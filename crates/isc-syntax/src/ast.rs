//! Abstract syntax tree, as produced by the parser.

use crate::lexer::Scalar;
use crate::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub decls: Vec<Decl>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Decl {
    Fn(FnDecl),
    Extern(ExternDecl),
    Type(TypeDecl),
    /// `import name` (D139): the file `name.isc`; its declarations are
    /// reached as `name.decl`.
    Import(String, Span),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Num,
    Ord,
    Eq,
}

impl Class {
    pub fn name(self) -> &'static str {
        match self {
            Class::Num => "Num",
            Class::Ord => "Ord",
            Class::Eq => "Eq",
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
