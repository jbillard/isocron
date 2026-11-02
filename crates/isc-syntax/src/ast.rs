//! Abstract syntax tree, as produced by the parser.

use crate::lexer::Scalar;
use crate::Span;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Program {
    pub decls: Vec<Decl>,
    /// The names this file exports (`export`): its functions, types and
    /// `declare function`s; a constructor is exported with its type.
    pub exports: Vec<String>,
    /// The names imported by name in each file once the modules are loaded
    /// (`import { f as g } from "./m";`): (file, local name, qualified
    /// name `m.f`). The type checker looks them up after the local
    /// variables and the module's own declarations.
    pub imported: Vec<(u32, String, String)>,
}

/// `import * as m from "./m";` or `import { f, g as h } from "./m";`.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    /// The module specifier: a path relative to the importing file
    /// (`./m`, `../lib/m`) or a name searched in `$ISC_PATH` (`m`).
    pub spec: String,
    pub span: Span,
    /// `* as m`: the prefix of the qualified names `m.x` of this file.
    pub alias: Option<String>,
    /// `{ f, g as h }`: (exported name, local name, position).
    pub names: Vec<(String, String, Span)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Fn(FnDecl),
    Extern(ExternDecl),
    Type(TypeDecl),
    /// An import (D139, D150).
    Import(Import),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Num,
    Ord,
    Eq,
    Key,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Num => "Num",
            Class::Ord => "Ord",
            Class::Eq => "Eq",
            Class::Key => "Key",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Generic {
    pub name: String,
    pub class: Option<Class>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParamType {
    Type(TypeExpr),
    Size,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: ParamType,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    RequireNoAlloc,
    RequireFused,
    RequireTail,
    AllowReassoc,
}

/// A `fn` or an `action` declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    pub is_action: bool,
    pub name: String,
    pub span: Span,
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
    pub modifiers: Vec<(Modifier, Span)>,
    pub body: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExternDecl {
    pub name: String,
    pub span: Span,
    pub params: Vec<Param>,
    pub ret: TypeExpr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<TypeExpr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeDecl {
    pub name: String,
    pub span: Span,
    pub generics: Vec<Generic>,
    pub variants: Vec<Variant>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SizeExpr {
    Lit(u64, Span),
    Var(String, Span),
    Add(Box<SizeExpr>, Box<SizeExpr>),
    Sub(Box<SizeExpr>, Box<SizeExpr>),
    Mul(Box<SizeExpr>, Box<SizeExpr>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    Scalar(Scalar),
    Str,
    /// `bigint` (D156).
    BigInt,
    Unit,
    Named(String, Vec<TypeExpr>),
    Array(SizeExpr, Box<TypeExpr>),
    BoundedArray(String, SizeExpr, Box<TypeExpr>),
    Tuple(Vec<TypeExpr>),
    Record(Vec<(String, TypeExpr)>),
    Fn(Vec<TypeExpr>, Box<TypeExpr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// `min(a, b)` and `max(a, b)` (no operator syntax: produced by the
    /// lowering of the primitives, SPEC section 10)
    Min,
    Max,
    /// Bitwise operators on integers (D136): `&`, `|`, `xor`, `<<`, `>>`
    /// (arithmetic for signed types, logical for unsigned ones); the shift
    /// amount is taken modulo the number of bits.
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Or => "||",
            BinOp::And => "&&",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Min => "min",
            BinOp::Max => "max",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
        }
    }
    pub fn is_bitwise(self) -> bool {
        matches!(self, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr)
    }
    pub fn is_arith(self) -> bool {
        matches!(self, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem)
    }
    pub fn is_order(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
    pub fn is_equality(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne)
    }
    pub fn is_logic(self) -> bool {
        matches!(self, BinOp::And | BinOp::Or)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Access {
    Index(Box<Expr>),
    Field(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Integer literal; `neg` is set when a unary minus was folded into it.
    Int { value: u128, neg: bool, suffix: Option<Scalar> },
    Float { value: f64, value32: f32, suffix: Option<Scalar> },
    /// A `bigint` literal (D156): its digits in `radix`, negated or not.
    BigInt { digits: String, radix: u32, neg: bool },
    Bool(bool),
    /// A string literal (D134).
    Str(String),
    Unit,
    Var(String),
    /// An operator used as a function: `(+)`.
    OpFn(BinOp),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    Record(Vec<(String, Expr)>),
    Call(Box<Expr>, Vec<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Pow(Box<Expr>, Box<Expr>),
    Update { target: Box<Expr>, access: Access, value: Box<Expr>, require_unique: bool },
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Match(Box<Expr>, Vec<(Pattern, Expr)>),
    Lambda(Vec<(String, Span)>, Box<Expr>),
    /// A sequence of statements ending with an expression (a `let` chain in a
    /// `fn`, or the body of an `action`).
    Block(Vec<Stmt>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Let(Pattern, Option<TypeExpr>, Expr, Span),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    Wild,
    Var(String),
    Int { value: u128, neg: bool },
    Float(f64, f32),
    Bool(bool),
    Unit,
    Tuple(Vec<Pattern>),
    Ctor(String, Vec<Pattern>),
    Record(Vec<(String, Pattern)>),
}

/// A visit of the names that can refer to a declaration: variables and
/// functions (`Var`), types (`Named`), constructors in patterns. The
/// modules use it to resolve the qualified names `m.x` (D150).
pub type NameVisitor<'a> = dyn FnMut(&mut String, Span) -> crate::Result<()> + 'a;

pub fn visit_names(p: &mut Program, f: &mut NameVisitor) -> crate::Result<()> {
    for d in p.decls.iter_mut() {
        match d {
            Decl::Fn(fd) => {
                for prm in fd.params.iter_mut() {
                    if let ParamType::Type(t) = &mut prm.ty {
                        visit_type(t, f)?;
                    }
                }
                visit_type(&mut fd.ret, f)?;
                visit_expr(&mut fd.body, f)?;
            }
            Decl::Extern(e) => {
                for prm in e.params.iter_mut() {
                    if let ParamType::Type(t) = &mut prm.ty {
                        visit_type(t, f)?;
                    }
                }
                visit_type(&mut e.ret, f)?;
            }
            Decl::Type(t) => {
                for v in t.variants.iter_mut() {
                    for x in v.fields.iter_mut() {
                        visit_type(x, f)?;
                    }
                }
            }
            Decl::Import(_) => {}
        }
    }
    Ok(())
}

fn visit_type(t: &mut TypeExpr, f: &mut NameVisitor) -> crate::Result<()> {
    match &mut t.kind {
        TypeKind::Named(n, args) => {
            f(n, t.span)?;
            for a in args.iter_mut() {
                visit_type(a, f)?;
            }
        }
        TypeKind::Array(_, e) | TypeKind::BoundedArray(_, _, e) => visit_type(e, f)?,
        TypeKind::Tuple(ts) => {
            for x in ts.iter_mut() {
                visit_type(x, f)?;
            }
        }
        TypeKind::Record(fs) => {
            for (_, x) in fs.iter_mut() {
                visit_type(x, f)?;
            }
        }
        TypeKind::Fn(ps, r) => {
            for x in ps.iter_mut() {
                visit_type(x, f)?;
            }
            visit_type(r, f)?;
        }
        TypeKind::Scalar(_) | TypeKind::Str | TypeKind::BigInt | TypeKind::Unit => {}
    }
    Ok(())
}

fn visit_pattern(p: &mut Pattern, f: &mut NameVisitor) -> crate::Result<()> {
    match &mut p.kind {
        PatternKind::Var(n) => f(n, p.span)?,
        PatternKind::Ctor(n, ps) => {
            f(n, p.span)?;
            for x in ps.iter_mut() {
                visit_pattern(x, f)?;
            }
        }
        PatternKind::Tuple(ps) => {
            for x in ps.iter_mut() {
                visit_pattern(x, f)?;
            }
        }
        PatternKind::Record(fs) => {
            for (_, x) in fs.iter_mut() {
                visit_pattern(x, f)?;
            }
        }
        PatternKind::Wild | PatternKind::Int { .. } | PatternKind::Float(..) | PatternKind::Bool(_) | PatternKind::Unit => {}
    }
    Ok(())
}

fn visit_expr(e: &mut Expr, f: &mut NameVisitor) -> crate::Result<()> {
    match &mut e.kind {
        ExprKind::Var(n) => f(n, e.span)?,
        ExprKind::Int { .. } | ExprKind::Float { .. } | ExprKind::BigInt { .. } | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Unit | ExprKind::OpFn(_) => {}
        ExprKind::Tuple(es) | ExprKind::Array(es) => {
            for x in es.iter_mut() {
                visit_expr(x, f)?;
            }
        }
        ExprKind::Record(fs) => {
            for (_, x) in fs.iter_mut() {
                visit_expr(x, f)?;
            }
        }
        ExprKind::Call(c, args) => {
            visit_expr(c, f)?;
            for x in args.iter_mut() {
                visit_expr(x, f)?;
            }
        }
        ExprKind::Index(a, b) | ExprKind::Binary(_, a, b) | ExprKind::Pow(a, b) => {
            visit_expr(a, f)?;
            visit_expr(b, f)?;
        }
        ExprKind::Slice(a, b, c) | ExprKind::If(a, b, c) => {
            visit_expr(a, f)?;
            visit_expr(b, f)?;
            visit_expr(c, f)?;
        }
        ExprKind::Field(a, _) | ExprKind::Unary(_, a) | ExprKind::Lambda(_, a) => visit_expr(a, f)?,
        ExprKind::Update { target, access, value, .. } => {
            visit_expr(target, f)?;
            if let Access::Index(i) = access {
                visit_expr(i, f)?;
            }
            visit_expr(value, f)?;
        }
        ExprKind::Match(s, arms) => {
            visit_expr(s, f)?;
            for (p, x) in arms.iter_mut() {
                visit_pattern(p, f)?;
                visit_expr(x, f)?;
            }
        }
        ExprKind::Block(stmts, last) => {
            for st in stmts.iter_mut() {
                match st {
                    Stmt::Let(p, t, x, _) => {
                        visit_pattern(p, f)?;
                        if let Some(t) = t {
                            visit_type(t, f)?;
                        }
                        visit_expr(x, f)?;
                    }
                    Stmt::Expr(x) => visit_expr(x, f)?,
                }
            }
            visit_expr(last, f)?;
        }
    }
    Ok(())
}
