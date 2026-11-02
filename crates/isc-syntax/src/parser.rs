//! The parser of Isocron (syntax inspired by TypeScript, DECISIONS D148):
//! the statements of a body (`const`, `if`, `return`) are lowered to the
//! expressions of the AST (`let` chains and `if` expressions), so that the
//! type checker and everything after it see one form.

use crate::ast::*;
use crate::lexer::{describe, lex, Scalar, Tok, S, T};
use crate::{Diagnostic, Result, Span};

/// Words that cannot name a variable or a function.
const RESERVED: &[&str] = &[
    "function", "action", "const", "let", "var", "return", "if", "else", "match", "type", "import", "export", "declare",
    "true", "false", "size", "with", "fn",
];

/// One statement of a body, before its lowering to the AST.
enum Stm {
    Const(Pattern, Option<TypeExpr>, Expr, Span),
    Return(Expr, Span),
    If(Expr, Vec<Stm>, Option<Vec<Stm>>, Span),
    Expr(Expr),
}

pub fn parse(src: &str, file: u32) -> Result<Program> {
    let toks = lex(src, file)?;
    let mut p = P { toks, pos: 0, modules: Vec::new(), in_action: false };
    p.program()
}

struct P {
    toks: Vec<Tok>,
    pos: usize,
    /// The prefixes `m` of `import * as m from "..."`: `m.x` is a qualified
    /// name, resolved when the modules are loaded (D150).
    modules: Vec<String>,
    in_action: bool,
}

// precedence levels (higher binds tighter), as in JavaScript
const L_OR: u8 = 2;
const L_AND: u8 = 3;
const L_BITOR: u8 = 4;
const L_BITXOR: u8 = 5;
const L_BITAND: u8 = 6;
const L_EQ: u8 = 7;
const L_REL: u8 = 8;
const L_SHIFT: u8 = 9;
const L_ADD: u8 = 10;
const L_MUL: u8 = 11;

impl P {
    fn peek(&self) -> &T {
        &self.toks[self.pos].t
    }
    fn peek_at(&self, k: usize) -> &T {
        &self.toks[(self.pos + k).min(self.toks.len() - 1)].t
    }
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn is(&self, s: S) -> bool {
        *self.peek() == T::Sym(s)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), T::Ident(x) if x == w)
    }
    fn eat(&mut self, s: S) -> bool {
        if self.is(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn error<X>(&self, msg: impl Into<String>) -> Result<X> {
        Err(Diagnostic::new(self.span(), msg))
    }
    fn expect(&mut self, s: S) -> Result<Span> {
        if self.is(s) {
            Ok(self.bump().span)
        } else {
            self.error(format!("expected `{}`, found {}", s.text(), describe(self.peek())))
        }
    }
    fn expect_word(&mut self, w: &str) -> Result<Span> {
        if self.is_word(w) {
            Ok(self.bump().span)
        } else {
            self.error(format!("expected `{}`, found {}", w, describe(self.peek())))
        }
    }
    fn ident(&mut self, what: &str) -> Result<(String, Span)> {
        match self.peek().clone() {
            T::Ident(s) if !RESERVED.contains(&s.as_str()) => {
                let sp = self.bump().span;
                Ok((s, sp))
            }
            t => self.error(format!("expected {}, found {}", what, describe(&t))),
        }
    }
    /// `m.x` of an imported module: the qualified name `m.x`, its prefix
    /// replaced by the module when the modules are loaded (D139, D150).
    fn module_of(&self, alias: &str) -> Option<String> {
        self.modules.iter().find(|a| *a == alias).cloned()
    }

    // ----- declarations ----------------------------------------------

    fn program(&mut self) -> Result<Program> {
        let mut decls = Vec::new();
        let mut exports = Vec::new();
        while *self.peek() != T::Eof {
            if self.is_word("import") {
                if !decls.iter().all(|d| matches!(d, Decl::Import(..))) {
                    return self.error("`import` comes before the other declarations");
                }
                decls.push(self.import()?);
                continue;
            }
            let mut modifiers = Vec::new();
            while self.is(S::At) {
                self.bump();
                let (m, sp) = self.ident("a decorator")?;
                let m = match m.as_str() {
                    "require_no_alloc" => Modifier::RequireNoAlloc,
                    "require_fused" => Modifier::RequireFused,
                    "require_tail" => Modifier::RequireTail,
                    "allow_reassoc" => Modifier::AllowReassoc,
                    _ => return Err(Diagnostic::new(sp, format!("unknown decorator `@{}`", m))),
                };
                modifiers.push((m, sp));
            }
            let exported = self.eat_word("export");
            let decl = if self.is_word("function") || self.is_word("action") {
                let is_action = self.is_word("action");
                if is_action && !modifiers.is_empty() {
                    return Err(Diagnostic::new(modifiers[0].1, "a decorator goes on a `function`, not on an `action`"));
                }
                Decl::Fn(self.function(is_action, modifiers)?)
            } else if !modifiers.is_empty() {
                return self.error("a decorator goes before `function`");
            } else if self.is_word("type") {
                Decl::Type(self.type_decl()?)
            } else if self.is_word("declare") {
                Decl::Extern(self.declare()?)
            } else if self.is_word("let") || self.is_word("const") {
                return self.error("a program declares functions, actions and types; `const` goes inside a body");
            } else {
                return self.error(format!(
                    "expected a declaration (`function`, `action`, `type`, `declare function` or `import`), found {}",
                    describe(self.peek())
                ));
            };
            if exported {
                match &decl {
                    Decl::Fn(f) => exports.push(f.name.clone()),
                    Decl::Extern(e) => exports.push(e.name.clone()),
                    Decl::Type(t) => {
                        exports.push(t.name.clone());
                        exports.extend(t.variants.iter().map(|v| v.name.clone()));
                    }
                    Decl::Import(_) => {}
                }
            }
            decls.push(decl);
        }
        Ok(Program { decls, exports, imported: Vec::new() })
    }

    /// `import * as m from "spec";` or `import { f, g as h } from "spec";`
    /// (D150): the module is found and read when the modules are loaded.
    fn import(&mut self) -> Result<Decl> {
        self.expect_word("import")?;
        let mut alias = None;
        let mut names = Vec::new();
        if self.eat(S::Star) {
            self.expect_word("as")?;
            let (a, sp) = self.ident("a module name")?;
            if self.modules.contains(&a) {
                return Err(Diagnostic::new(sp, format!("`{}` names two imported modules", a)));
            }
            self.modules.push(a.clone());
            alias = Some(a);
        } else if self.eat(S::LBrace) {
            while !self.is(S::RBrace) {
                let (n, sp) = self.ident("an imported name")?;
                let local = if self.eat_word("as") { self.ident("a local name")?.0 } else { n.clone() };
                if names.iter().any(|(_, l, _): &(String, String, Span)| *l == local) {
                    return Err(Diagnostic::new(sp, format!("`{}` is imported twice", local)));
                }
                names.push((n, local, sp));
                if !self.eat(S::Comma) {
                    break;
                }
            }
            self.expect(S::RBrace)?;
        } else {
            return self.error("expected `import * as name from \"...\";` or `import { a, b } from \"...\";`");
        }
        self.expect_word("from")?;
        let span = self.span();
        let spec = match self.peek().clone() {
            T::Str(s) => {
                self.bump();
                s
            }
            t => return self.error(format!("expected the path of the module, a string, found {}", describe(&t))),
        };
        self.expect(S::Semi)?;
        Ok(Decl::Import(Import { spec, span, alias, names }))
    }

    fn generics(&mut self) -> Result<Vec<Generic>> {
        let mut gs = Vec::new();
        if !self.eat(S::Lt) {
            return Ok(gs);
        }
        loop {
            let (name, span) = self.ident("a type parameter")?;
            let class = if self.eat_word("extends") {
                let (c, sp) = self.ident("`Num`, `Ord` or `Eq`")?;
                Some(match c.as_str() {
                    "Num" => Class::Num,
                    "Ord" => Class::Ord,
                    "Eq" => Class::Eq,
                    _ => return Err(Diagnostic::new(sp, format!("unknown class `{}` (`Num`, `Ord` or `Eq`)", c))),
                })
            } else {
                None
            };
            gs.push(Generic { name, class, span });
            if !self.eat(S::Comma) {
                break;
            }
        }
        self.expect(S::Gt)?;
        Ok(gs)
    }

    fn params(&mut self) -> Result<Vec<Param>> {
        self.expect(S::LParen)?;
        let mut ps = Vec::new();
        if !self.is(S::RParen) {
            loop {
                let (name, span) = self.ident("a parameter name")?;
                self.expect(S::Colon)?;
                let ty = if self.eat_word("size") { ParamType::Size } else { ParamType::Type(self.ty()?) };
                ps.push(Param { name, ty, span });
                if !self.eat(S::Comma) {
                    break;
                }
            }
        }
        self.expect(S::RParen)?;
        Ok(ps)
    }

    fn function(&mut self, is_action: bool, modifiers: Vec<(Modifier, Span)>) -> Result<FnDecl> {
        self.bump();
        let (name, span) = self.ident("a function name")?;
        let generics = self.generics()?;
        if is_action && !generics.is_empty() {
            return Err(Diagnostic::new(span, "an action has no type parameters"));
        }
        let params = self.params()?;
        self.expect(S::Colon)?;
        let ret = self.ty()?;
        let saved = self.in_action;
        self.in_action = is_action;
        let lb = self.span();
        let stmts = self.block()?;
        // as in TypeScript, a value is given by `return`: an action whose
        // result is not `void` ends with `return` on every path
        if is_action && !matches!(ret.kind, TypeKind::Unit) && !Self::returns(&stmts) {
            return Err(Diagnostic::new(span, format!("the action `{}` returns a value: its body ends with `return` on every path", name)));
        }
        let body = self.lower(&stmts, is_action, lb);
        self.in_action = saved;
        Ok(FnDecl { is_action, name, span, generics, params, ret, modifiers, body: body? })
    }

    fn type_decl(&mut self) -> Result<TypeDecl> {
        self.expect_word("type")?;
        let (name, span) = self.ident("a type name")?;
        let generics = self.generics()?;
        self.expect(S::Assign)?;
        self.eat(S::Bar);
        let mut variants = Vec::new();
        loop {
            let (vname, vspan) = self.ident("a constructor")?;
            let mut fields = Vec::new();
            if self.eat(S::LParen) {
                loop {
                    fields.push(self.ty()?);
                    if !self.eat(S::Comma) {
                        break;
                    }
                }
                self.expect(S::RParen)?;
            }
            variants.push(Variant { name: vname, fields, span: vspan });
            if !self.eat(S::Bar) {
                break;
            }
        }
        self.expect(S::Semi)?;
        Ok(TypeDecl { name, span, generics, variants })
    }

    fn declare(&mut self) -> Result<ExternDecl> {
        self.expect_word("declare")?;
        self.expect_word("function")?;
        let (name, span) = self.ident("a function name")?;
        let params = self.params()?;
        self.expect(S::Colon)?;
        let ret = self.ty()?;
        self.expect(S::Semi)?;
        Ok(ExternDecl { name, span, params, ret })
    }

    // ----- types -----------------------------------------------------

    fn ty(&mut self) -> Result<TypeExpr> {
        let span = self.span();
        let mut base = self.base_ty()?;
        // `T[n]`, `T[r][c]` (r rows of c), `T[k <= n]`
        let mut sizes: Vec<(Option<String>, SizeExpr)> = Vec::new();
        while self.is(S::LBracket) {
            self.bump();
            if let (T::Ident(k), T::Sym(S::Le)) = (self.peek().clone(), self.peek_at(1).clone()) {
                self.bump();
                self.bump();
                let n = self.size_expr()?;
                sizes.push((Some(k), n));
            } else {
                sizes.push((None, self.size_expr()?));
            }
            self.expect(S::RBracket)?;
        }
        for (k, n) in sizes.into_iter().rev() {
            base = TypeExpr {
                kind: match k {
                    Some(k) => TypeKind::BoundedArray(k, n, Box::new(base)),
                    None => TypeKind::Array(n, Box::new(base)),
                },
                span,
            };
        }
        Ok(base)
    }

    fn base_ty(&mut self) -> Result<TypeExpr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            T::Ident(name) => {
                self.bump();
                let name = match self.module_of(&name) {
                    Some(m) if self.is(S::Dot) => {
                        self.bump();
                        format!("{}.{}", m, self.ident("a type of the module")?.0)
                    }
                    _ => name,
                };
                match name.as_str() {
                    "boolean" => TypeKind::Scalar(Scalar::Bool),
                    "bool" => return Err(Diagnostic::new(span, "the type of `true` and `false` is written `boolean`")),
                    "string" => TypeKind::Str,
                    "void" => TypeKind::Unit,
                    n if Scalar::from_name(n).is_some() => TypeKind::Scalar(Scalar::from_name(n).unwrap()),
                    _ => {
                        let mut args = Vec::new();
                        if self.eat(S::Lt) {
                            loop {
                                args.push(self.ty()?);
                                if !self.eat(S::Comma) {
                                    break;
                                }
                            }
                            self.expect(S::Gt)?;
                        }
                        TypeKind::Named(name, args)
                    }
                }
            }
            T::Sym(S::LBracket) => {
                self.bump();
                let mut ts = Vec::new();
                if !self.is(S::RBracket) {
                    loop {
                        ts.push(self.ty()?);
                        if !self.eat(S::Comma) {
                            break;
                        }
                    }
                }
                self.expect(S::RBracket)?;
                TypeKind::Tuple(ts)
            }
            T::Sym(S::LBrace) => {
                self.bump();
                let mut fs = Vec::new();
                loop {
                    let (f, _) = self.ident("a field name")?;
                    self.expect(S::Colon)?;
                    fs.push((f, self.ty()?));
                    if !self.eat(S::Comma) || self.is(S::RBrace) {
                        break;
                    }
                }
                self.expect(S::RBrace)?;
                TypeKind::Record(fs)
            }
            T::Sym(S::LParen) => {
                // `(A, B) => C`, `(x: A) => C`, or `(T)`
                self.bump();
                let mut ps = Vec::new();
                if !self.is(S::RParen) {
                    loop {
                        if let (T::Ident(_), T::Sym(S::Colon)) = (self.peek().clone(), self.peek_at(1).clone()) {
                            self.bump();
                            self.bump();
                        }
                        ps.push(self.ty()?);
                        if !self.eat(S::Comma) {
                            break;
                        }
                    }
                }
                self.expect(S::RParen)?;
                if self.eat(S::FatArrow) {
                    let r = self.ty()?;
                    TypeKind::Fn(ps, Box::new(r))
                } else if ps.len() == 1 {
                    return Ok(ps.pop().unwrap());
                } else {
                    return self.error("expected `=>` after the parameters of a function type");
                }
            }
            t => return self.error(format!("expected a type, found {}", describe(&t))),
        };
        Ok(TypeExpr { kind, span })
    }

    fn size_expr(&mut self) -> Result<SizeExpr> {
        let mut a = self.size_term()?;
        loop {
            if self.eat(S::Plus) {
                let b = self.size_term()?;
                a = SizeExpr::Add(Box::new(a), Box::new(b));
            } else if self.eat(S::Minus) {
                let b = self.size_term()?;
                a = SizeExpr::Sub(Box::new(a), Box::new(b));
            } else {
                return Ok(a);
            }
        }
    }

    fn size_term(&mut self) -> Result<SizeExpr> {
        let mut a = self.size_atom()?;
        while self.eat(S::Star) {
            let b = self.size_atom()?;
            a = SizeExpr::Mul(Box::new(a), Box::new(b));
        }
        Ok(a)
    }

    fn size_atom(&mut self) -> Result<SizeExpr> {
        let span = self.span();
        match self.peek().clone() {
            T::Int { value, suffix: None } => {
                self.bump();
                Ok(SizeExpr::Lit(value as u64, span))
            }
            T::Ident(n) => {
                self.bump();
                Ok(SizeExpr::Var(n, span))
            }
            T::Sym(S::LParen) => {
                self.bump();
                let e = self.size_expr()?;
                self.expect(S::RParen)?;
                Ok(e)
            }
            t => self.error(format!("expected a size, found {}", describe(&t))),
        }
    }

    // ----- bodies ----------------------------------------------------

    fn block(&mut self) -> Result<Vec<Stm>> {
        self.expect(S::LBrace)?;
        let mut v = Vec::new();
        while !self.is(S::RBrace) {
            if *self.peek() == T::Eof {
                return self.error("expected `}` at the end of the block");
            }
            v.push(self.statement()?);
        }
        self.bump();
        Ok(v)
    }

    fn statement(&mut self) -> Result<Stm> {
        let span = self.span();
        if self.is_word("let") || self.is_word("var") {
            return self.error("a variable cannot change: write `const` (D148)");
        }
        if self.eat_word("const") {
            let pat = self.pattern()?;
            let ty = if self.eat(S::Colon) { Some(self.ty()?) } else { None };
            self.expect(S::Assign)?;
            let e = self.expr()?;
            self.expect(S::Semi)?;
            return Ok(Stm::Const(pat, ty, e, span));
        }
        if self.eat_word("return") {
            let e = if self.is(S::Semi) { Expr { kind: ExprKind::Unit, span } } else { self.expr()? };
            self.expect(S::Semi)?;
            return Ok(Stm::Return(e, span));
        }
        if self.eat_word("if") {
            self.expect(S::LParen)?;
            let c = self.expr()?;
            self.expect(S::RParen)?;
            let t = self.branch()?;
            let f = if self.eat_word("else") { Some(self.branch()?) } else { None };
            return Ok(Stm::If(c, t, f, span));
        }
        if self.is(S::LBrace) {
            return self.error("a block `{ ... }` alone is not a statement");
        }
        let e = self.expr()?;
        self.expect(S::Semi)?;
        Ok(Stm::Expr(e))
    }

    /// The branch of an `if` statement: a block, or one statement.
    fn branch(&mut self) -> Result<Vec<Stm>> {
        if self.is(S::LBrace) {
            self.block()
        } else {
            Ok(vec![self.statement()?])
        }
    }

    /// Whether every path of a list of statements ends with `return`.
    fn returns(stmts: &[Stm]) -> bool {
        match stmts.last() {
            Some(Stm::Return(..)) => true,
            Some(Stm::If(_, t, Some(f), _)) => Self::returns(t) && Self::returns(f),
            _ => false,
        }
    }

    fn has_return(stmts: &[Stm]) -> bool {
        stmts.iter().any(|s| match s {
            Stm::Return(..) => true,
            Stm::If(_, t, f, _) => Self::has_return(t) || f.as_ref().map(|f| Self::has_return(f)).unwrap_or(false),
            _ => false,
        })
    }

    /// The expression of a body: `const`s are `let`s, an `if` with a
    /// `return` decides the value (the rest of the body continues the
    /// branch that does not return); without `return`, an action is
    /// `void` (D148, D149).
    fn lower(&self, stmts: &[Stm], action: bool, span: Span) -> Result<Expr> {
        self.lower_owned(&stmts.iter().collect::<Vec<_>>(), action, span)
    }

    fn lower_owned(&self, stmts: &[&Stm], action: bool, span: Span) -> Result<Expr> {
        let mut out: Vec<Stmt> = Vec::new();
        let mut last: Option<Expr> = None;
        let mut i = 0;
        while i < stmts.len() {
            let rest = &stmts[i + 1..];
            match stmts[i] {
                Stm::Const(p, t, e, sp) => out.push(Stmt::Let(p.clone(), t.clone(), e.clone(), *sp)),
                Stm::Return(e, sp) => {
                    if !rest.is_empty() {
                        return Err(Diagnostic::new(*sp, "the statements after this `return` are never run"));
                    }
                    last = Some(e.clone());
                }
                // never the value, even the last one (the value of an
                // action without `return` is `void`)
                Stm::Expr(e) => out.push(Stmt::Expr(e.clone())),
                Stm::If(c, t, f, sp) => {
                    let has = Self::has_return(t) || f.as_ref().map(|f| Self::has_return(f)).unwrap_or(false);
                    if has {
                        // the value: each branch with the rest of the body
                        // where it does not return
                        let tr = Self::returns(t);
                        let fr = f.as_ref().map(|f| Self::returns(f)).unwrap_or(false);
                        if tr && fr && !rest.is_empty() {
                            return Err(Diagnostic::new(*sp, "the statements after this `if` are never run"));
                        }
                        let mut tv: Vec<&Stm> = t.iter().collect();
                        if !tr {
                            tv.extend(rest.iter().copied());
                        }
                        let te = self.lower_owned(&tv, action, *sp)?;
                        let fe = match f {
                            Some(f) => {
                                let mut fv: Vec<&Stm> = f.iter().collect();
                                if !fr {
                                    fv.extend(rest.iter().copied());
                                }
                                self.lower_owned(&fv, action, *sp)?
                            }
                            None => self.lower_owned(rest, action, *sp)?,
                        };
                        last = Some(Expr { kind: ExprKind::If(Box::new(c.clone()), Box::new(te), Box::new(fe)), span: *sp });
                        break;
                    }
                    // a statement of an action: each branch its own value
                    let te = self.lower(t, action, *sp)?;
                    let fe = match f {
                        Some(f) => self.lower(f, action, *sp)?,
                        None => Expr { kind: ExprKind::Unit, span: *sp },
                    };
                    let x = Expr { kind: ExprKind::If(Box::new(c.clone()), Box::new(te), Box::new(fe)), span: *sp };
                    if rest.is_empty() {
                        last = Some(x);
                    } else {
                        out.push(Stmt::Expr(x));
                    }
                }
            }
            i += 1;
        }
        let last = match last {
            Some(e) => e,
            None if action => Expr { kind: ExprKind::Unit, span },
            None => return Err(Diagnostic::new(span, "a function ends with `return`")),
        };
        Ok(if out.is_empty() { last } else { Expr { kind: ExprKind::Block(out, Box::new(last)), span } })
    }

    // ----- expressions ---------------------------------------------

    fn expr(&mut self) -> Result<Expr> {
        // a lambda: `(a, b) => ...`, `x => ...`
        if let T::Ident(n) = self.peek().clone() {
            if *self.peek_at(1) == T::Sym(S::FatArrow) && !RESERVED.contains(&n.as_str()) {
                let span = self.bump().span;
                self.bump();
                return self.lambda_body(vec![(n, span)], Vec::new(), span);
            }
        }
        if self.is(S::LParen) && self.lambda_ahead() {
            return self.lambda();
        }
        let span = self.span();
        let c = self.binary(L_OR)?;
        if self.eat(S::Question) {
            let t = self.expr()?;
            self.expect(S::Colon)?;
            let f = self.expr()?;
            return Ok(Expr { kind: ExprKind::If(Box::new(c), Box::new(t), Box::new(f)), span });
        }
        Ok(c)
    }

    /// After `(`, the matching `)` is followed by `=>`.
    fn lambda_ahead(&self) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos;
        while i < self.toks.len() {
            match self.toks[i].t {
                T::Sym(S::LParen) | T::Sym(S::LBracket) | T::Sym(S::LBrace) => depth += 1,
                T::Sym(S::RParen) | T::Sym(S::RBracket) | T::Sym(S::RBrace) => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.toks.get(i + 1).map(|t| &t.t), Some(T::Sym(S::FatArrow)));
                    }
                }
                T::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn lambda(&mut self) -> Result<Expr> {
        let span = self.expect(S::LParen)?;
        let mut ps = Vec::new();
        // typed parameters: a `const` of the annotated type in the body
        let mut typed = Vec::new();
        if !self.is(S::RParen) {
            loop {
                let (n, sp) = self.ident("a parameter name")?;
                if self.eat(S::Colon) {
                    let t = self.ty()?;
                    typed.push((n.clone(), sp, t));
                }
                ps.push((n, sp));
                if !self.eat(S::Comma) {
                    break;
                }
            }
        }
        self.expect(S::RParen)?;
        if self.eat(S::Colon) {
            return self.error("the result type of a lambda is inferred");
        }
        self.expect(S::FatArrow)?;
        self.lambda_body(ps, typed, span)
    }

    fn lambda_body(&mut self, ps: Vec<(String, Span)>, typed: Vec<(String, Span, TypeExpr)>, span: Span) -> Result<Expr> {
        let body = if self.is(S::LBrace) {
            let lb = self.span();
            let stmts = self.block()?;
            // a lambda is pure (it cannot call an action)
            self.lower(&stmts, false, lb)?
        } else {
            self.expr()?
        };
        let body = if typed.is_empty() {
            body
        } else {
            let lets = typed
                .into_iter()
                .map(|(n, sp, t)| {
                    Stmt::Let(Pattern { kind: PatternKind::Var(n.clone()), span: sp }, Some(t), Expr { kind: ExprKind::Var(n), span: sp }, sp)
                })
                .collect();
            Expr { kind: ExprKind::Block(lets, Box::new(body)), span }
        };
        Ok(Expr { kind: ExprKind::Lambda(ps, Box::new(body)), span })
    }

    /// The binary operator here, its level and its number of tokens (`<<`
    /// and `>>` are two adjacent `<` or `>`, so that `>>` still closes two
    /// lists of type arguments).
    fn binop(&self) -> Option<(BinOp, u8, usize)> {
        let adjacent = |s: S| {
            let a = &self.toks[self.pos];
            let b = &self.toks[(self.pos + 1).min(self.toks.len() - 1)];
            b.t == T::Sym(s) && b.span.line == a.span.line && b.span.col == a.span.col + 1
        };
        Some(match self.peek() {
            T::Sym(S::BarBar) => (BinOp::Or, L_OR, 1),
            T::Sym(S::AmpAmp) => (BinOp::And, L_AND, 1),
            T::Sym(S::Bar) => (BinOp::BitOr, L_BITOR, 1),
            T::Sym(S::Caret) => (BinOp::BitXor, L_BITXOR, 1),
            T::Sym(S::Amp) => (BinOp::BitAnd, L_BITAND, 1),
            T::Sym(S::EqEq) | T::Sym(S::EqEqEq) => (BinOp::Eq, L_EQ, 1),
            T::Sym(S::NotEq) | T::Sym(S::NotEqEq) => (BinOp::Ne, L_EQ, 1),
            T::Sym(S::Lt) if adjacent(S::Lt) => (BinOp::Shl, L_SHIFT, 2),
            T::Sym(S::Gt) if adjacent(S::Gt) => (BinOp::Shr, L_SHIFT, 2),
            T::Sym(S::Lt) => (BinOp::Lt, L_REL, 1),
            T::Sym(S::Le) => (BinOp::Le, L_REL, 1),
            T::Sym(S::Gt) => (BinOp::Gt, L_REL, 1),
            T::Sym(S::Ge) => (BinOp::Ge, L_REL, 1),
            T::Sym(S::Plus) => (BinOp::Add, L_ADD, 1),
            T::Sym(S::Minus) => (BinOp::Sub, L_ADD, 1),
            T::Sym(S::Star) => (BinOp::Mul, L_MUL, 1),
            T::Sym(S::Slash) => (BinOp::Div, L_MUL, 1),
            T::Sym(S::Percent) => (BinOp::Rem, L_MUL, 1),
            _ => return None,
        })
    }

    fn binary(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.power()?;
        while let Some((op, p, len)) = self.binop() {
            if p < min {
                break;
            }
            let span = self.bump().span;
            if len == 2 {
                self.bump();
            }
            let rhs = self.binary(p + 1)?;
            if p == L_EQ || p == L_REL {
                if let Some((op2, p2, _)) = self.binop() {
                    if p2 == p {
                        return self.error(format!(
                            "comparison operators cannot be chained: `{}` after `{}`; use parentheses",
                            op2.text(),
                            op.text()
                        ));
                    }
                }
            }
            lhs = Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), span };
        }
        Ok(lhs)
    }

    /// `a ** b`, right associative; its left side is not a unary operation
    /// (`-x ** 2` is an error, as in TypeScript: write `-(x ** 2)`).
    fn power(&mut self) -> Result<Expr> {
        let start = self.pos;
        let base = self.unary()?;
        if self.is(S::StarStar) {
            if matches!(self.toks[start].t, T::Sym(S::Minus) | T::Sym(S::Bang)) {
                return self.error("parenthesize the left side of `**` (`-x ** 2` is `-(x ** 2)` or `(-x) ** 2`)");
            }
            let span = self.bump().span;
            let exp = self.power_right()?;
            return Ok(Expr { kind: ExprKind::Pow(Box::new(base), Box::new(exp)), span });
        }
        Ok(base)
    }

    fn power_right(&mut self) -> Result<Expr> {
        let base = self.unary()?;
        if self.is(S::StarStar) {
            let span = self.bump().span;
            let exp = self.power_right()?;
            return Ok(Expr { kind: ExprKind::Pow(Box::new(base), Box::new(exp)), span });
        }
        Ok(base)
    }

    fn unary(&mut self) -> Result<Expr> {
        let span = self.span();
        if self.eat(S::Minus) {
            let e = self.unary()?;
            if let ExprKind::Int { value, neg: false, suffix } = e.kind {
                return Ok(Expr { kind: ExprKind::Int { value, neg: true, suffix }, span });
            }
            if let ExprKind::Float { value, value32, suffix } = e.kind {
                return Ok(Expr { kind: ExprKind::Float { value: -value, value32: -value32, suffix }, span });
            }
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Neg, Box::new(e)), span });
        }
        if self.eat(S::Bang) {
            let e = self.unary()?;
            return Ok(Expr { kind: ExprKind::Unary(UnOp::Not, Box::new(e)), span });
        }
        self.postfix()
    }

    fn args(&mut self, close: S) -> Result<Vec<Expr>> {
        let mut v = Vec::new();
        if !self.is(close) {
            loop {
                v.push(self.expr()?);
                if !self.eat(S::Comma) || self.is(close) {
                    break;
                }
            }
        }
        self.expect(close)?;
        Ok(v)
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.is(S::LParen) {
                let span = self.bump().span;
                let args = self.args(S::RParen)?;
                e = Expr { kind: ExprKind::Call(Box::new(e), args), span };
            } else if self.is(S::LBracket) {
                let span = self.bump().span;
                let i = self.expr()?;
                self.expect(S::RBracket)?;
                e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(i)), span };
            } else if self.is(S::Dot) {
                let span = self.bump().span;
                let (f, fsp) = match self.peek().clone() {
                    T::Ident(f) => {
                        let sp = self.bump().span;
                        (f, sp)
                    }
                    t => return self.error(format!("expected a field or a method, found {}", describe(&t))),
                };
                // `m.x` of an imported module (D139)
                if let ExprKind::Var(m) = &e.kind {
                    if let Some(name) = self.module_of(m) {
                        e = Expr { kind: ExprKind::Var(format!("{}.{}", name, f)), span: e.span };
                        continue;
                    }
                }
                match f.as_str() {
                    // `x.length`: `len(x)`
                    "length" if !self.is(S::LParen) => {
                        let callee = Expr { kind: ExprKind::Var("len".into()), span: fsp };
                        e = Expr { kind: ExprKind::Call(Box::new(callee), vec![e]), span };
                    }
                    // `x.slice(a, b)`: the slice `x[a..b]`
                    "slice" if self.is(S::LParen) => {
                        let sp = self.bump().span;
                        let mut a = self.args(S::RParen)?;
                        if a.len() != 2 {
                            return Err(Diagnostic::new(sp, "`slice` takes a start and an end"));
                        }
                        let b = a.pop().unwrap();
                        let a = a.pop().unwrap();
                        e = Expr { kind: ExprKind::Slice(Box::new(e), Box::new(a), Box::new(b)), span: sp };
                    }
                    // `x.with(i, v)`: the update `x with [i] = v`
                    "with" if self.is(S::LParen) => {
                        let sp = self.bump().span;
                        let mut a = self.args(S::RParen)?;
                        if a.len() != 2 {
                            return Err(Diagnostic::new(sp, "`with` takes an index and a value"));
                        }
                        let v = a.pop().unwrap();
                        let i = a.pop().unwrap();
                        e = Expr {
                            kind: ExprKind::Update {
                                target: Box::new(e),
                                access: Access::Index(Box::new(i)),
                                value: Box::new(v),
                                require_unique: false,
                            },
                            span: sp,
                        };
                    }
                    // a field, or a method `x.m(...)`: `m(x, ...)`, told
                    // apart by the type checker
                    _ => e = Expr { kind: ExprKind::Field(Box::new(e), f), span },
                }
            } else if self.is_word("as") && matches!(self.peek_at(1), T::Ident(c) if c == "const") {
                // `[a, b] as const`: a tuple
                let sp = self.bump().span;
                self.bump();
                match e.kind {
                    ExprKind::Array(es) => e = Expr { kind: ExprKind::Tuple(es), span: e.span },
                    _ => return Err(Diagnostic::new(sp, "`as const` follows a literal `[a, b, ...]` (a tuple)")),
                }
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            T::Int { value, suffix } => {
                self.bump();
                ExprKind::Int { value, neg: false, suffix }
            }
            T::Float { value, value32, suffix } => {
                self.bump();
                ExprKind::Float { value, value32, suffix }
            }
            T::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            T::Ident(w) if w == "true" || w == "false" => {
                self.bump();
                ExprKind::Bool(w == "true")
            }
            T::Ident(w) if w == "match" => return self.match_expr(),
            T::Ident(w) if w == "require_unique" && *self.peek_at(1) == T::Sym(S::LParen) => {
                self.bump();
                self.bump();
                let e = self.expr()?;
                self.expect(S::RParen)?;
                return match e.kind {
                    ExprKind::Update { target, access, value, .. } => {
                        Ok(Expr { kind: ExprKind::Update { target, access, value, require_unique: true }, span: e.span })
                    }
                    _ => Err(Diagnostic::new(span, "`require_unique(...)` takes an update `x.with(i, v)`")),
                };
            }
            T::Ident(w) if RESERVED.contains(&w.as_str()) => {
                return self.error(format!("expected an expression, found the keyword `{}`", w));
            }
            T::Ident(w) => {
                self.bump();
                ExprKind::Var(w)
            }
            T::Sym(S::LParen) => {
                self.bump();
                if self.eat(S::RParen) {
                    ExprKind::Unit
                } else if let (Some((op, _, len)), true) = (self.binop(), self.op_section_ahead()) {
                    // `(+)`: the operator as a function
                    self.bump();
                    if len == 2 {
                        self.bump();
                    }
                    self.expect(S::RParen)?;
                    ExprKind::OpFn(op)
                } else {
                    let e = self.expr()?;
                    self.expect(S::RParen)?;
                    return Ok(e);
                }
            }
            T::Sym(S::LBracket) => {
                self.bump();
                ExprKind::Array(self.args(S::RBracket)?)
            }
            T::Sym(S::LBrace) => return self.record(),
            T::Sym(S::Backtick) => return self.template(),
            t => return self.error(format!("expected an expression, found {}", describe(&t))),
        };
        Ok(Expr { kind, span })
    }

    /// `` `a${e}b` `` (D150): the concatenation of the texts and of the
    /// insertions, each written as `print` writes it (`$text`, a name no
    /// program can declare).
    fn template(&mut self) -> Result<Expr> {
        let span = self.expect(S::Backtick)?;
        let mut parts: Vec<Expr> = Vec::new();
        loop {
            let (text, sp) = match self.peek().clone() {
                T::Str(t) => (t, self.bump().span),
                t => return self.error(format!("expected the text of a template literal, found {}", describe(&t))),
            };
            if !text.is_empty() {
                parts.push(Expr { kind: ExprKind::Str(text), span: sp });
            }
            if self.eat(S::Backtick) {
                break;
            }
            self.expect(S::TplOpen)?;
            let e = self.expr()?;
            self.expect(S::TplClose)?;
            let callee = Expr { kind: ExprKind::Var("$text".into()), span: e.span };
            parts.push(Expr { span: e.span, kind: ExprKind::Call(Box::new(callee), vec![e]) });
        }
        let mut it = parts.into_iter();
        let mut acc = match it.next() {
            Some(e) => e,
            None => return Ok(Expr { kind: ExprKind::Str(String::new()), span }),
        };
        if !matches!(acc.kind, ExprKind::Str(_)) && it.len() == 0 {
            // `${e}` alone is the text of `e`
            return Ok(acc);
        }
        for e in it {
            acc = Expr { span, kind: ExprKind::Binary(BinOp::Add, Box::new(acc), Box::new(e)) };
        }
        Ok(acc)
    }

    /// `(op)`: an operator alone between parentheses.
    fn op_section_ahead(&self) -> bool {
        let Some((_, _, len)) = self.binop() else { return false };
        *self.peek_at(len) == T::Sym(S::RParen)
    }

    /// `{ a: e, b: e }`, or `{ ...r, f: v }` (an update of `r`).
    fn record(&mut self) -> Result<Expr> {
        let span = self.expect(S::LBrace)?;
        if self.eat(S::Ellipsis) {
            let target = self.expr()?;
            self.expect(S::Comma)?;
            let mut e = target;
            loop {
                let (f, fsp) = self.ident("a field name")?;
                self.expect(S::Colon)?;
                let v = self.expr()?;
                e = Expr { kind: ExprKind::Update { target: Box::new(e), access: Access::Field(f), value: Box::new(v), require_unique: false }, span: fsp };
                if !self.eat(S::Comma) || self.is(S::RBrace) {
                    break;
                }
            }
            self.expect(S::RBrace)?;
            return Ok(e);
        }
        let mut fields = Vec::new();
        loop {
            let (f, fsp) = self.ident("a field name")?;
            if self.eat(S::Colon) {
                fields.push((f, self.expr()?));
            } else {
                // `{ a }`: the field `a` of the variable `a`
                fields.push((f.clone(), Expr { kind: ExprKind::Var(f), span: fsp }));
            }
            if !self.eat(S::Comma) || self.is(S::RBrace) {
                break;
            }
        }
        self.expect(S::RBrace)?;
        Ok(Expr { kind: ExprKind::Record(fields), span })
    }

    fn match_expr(&mut self) -> Result<Expr> {
        let span = self.expect_word("match")?;
        self.expect(S::LParen)?;
        let scrut = self.expr()?;
        self.expect(S::RParen)?;
        self.expect(S::LBrace)?;
        let mut arms = Vec::new();
        while !self.is(S::RBrace) {
            let p = self.pattern()?;
            self.expect(S::FatArrow)?;
            let body = if self.is(S::LBrace) {
                let lb = self.span();
                let stmts = self.block()?;
                self.lower(&stmts, self.in_action, lb)?
            } else {
                self.expr()?
            };
            arms.push((p, body));
            if !self.eat(S::Comma) {
                break;
            }
        }
        self.expect(S::RBrace)?;
        if arms.is_empty() {
            return Err(Diagnostic::new(span, "a `match` has at least one arm"));
        }
        Ok(Expr { kind: ExprKind::Match(Box::new(scrut), arms), span })
    }

    fn pattern(&mut self) -> Result<Pattern> {
        let span = self.span();
        let kind = match self.peek().clone() {
            T::Ident(w) if w == "_" => {
                self.bump();
                PatternKind::Wild
            }
            T::Ident(w) if w == "true" || w == "false" => {
                self.bump();
                PatternKind::Bool(w == "true")
            }
            T::Ident(name) => {
                self.bump();
                let name = match self.module_of(&name) {
                    Some(m) if self.is(S::Dot) => {
                        self.bump();
                        format!("{}.{}", m, self.ident("a constructor of the module")?.0)
                    }
                    _ => name,
                };
                if self.eat(S::LParen) {
                    let mut ps = Vec::new();
                    if !self.is(S::RParen) {
                        loop {
                            ps.push(self.pattern()?);
                            if !self.eat(S::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(S::RParen)?;
                    PatternKind::Ctor(name, ps)
                } else {
                    PatternKind::Var(name)
                }
            }
            T::Int { value, .. } => {
                self.bump();
                PatternKind::Int { value, neg: false }
            }
            T::Float { value, value32, .. } => {
                self.bump();
                PatternKind::Float(value, value32)
            }
            T::Sym(S::Minus) => {
                self.bump();
                match self.peek().clone() {
                    T::Int { value, .. } => {
                        self.bump();
                        PatternKind::Int { value, neg: true }
                    }
                    T::Float { value, value32, .. } => {
                        self.bump();
                        PatternKind::Float(-value, -value32)
                    }
                    t => return self.error(format!("expected a number after `-`, found {}", describe(&t))),
                }
            }
            T::Sym(S::LParen) => {
                self.bump();
                self.expect(S::RParen)?;
                PatternKind::Unit
            }
            T::Sym(S::LBracket) => {
                self.bump();
                let mut ps = Vec::new();
                if !self.is(S::RBracket) {
                    loop {
                        ps.push(self.pattern()?);
                        if !self.eat(S::Comma) {
                            break;
                        }
                    }
                }
                self.expect(S::RBracket)?;
                PatternKind::Tuple(ps)
            }
            T::Sym(S::LBrace) => {
                self.bump();
                let mut fs = Vec::new();
                loop {
                    let (f, fsp) = self.ident("a field name")?;
                    let p = if self.eat(S::Colon) { self.pattern()? } else { Pattern { kind: PatternKind::Var(f.clone()), span: fsp } };
                    fs.push((f, p));
                    if !self.eat(S::Comma) || self.is(S::RBrace) {
                        break;
                    }
                }
                self.expect(S::RBrace)?;
                PatternKind::Record(fs)
            }
            t => return self.error(format!("expected a pattern, found {}", describe(&t))),
        };
        Ok(Pattern { kind, span })
    }
}
