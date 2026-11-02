//! `require_fused` and `require_no_alloc` (SPEC section 9, DECISIONS
//! D106), checked on the IR after fusion and uniqueness. Each function with
//! one of them is lowered alone, once per type arguments it is used with
//! (`lower::probe`): its parameters are inputs it borrows and its result is
//! the destination its caller gives (section 8). Its memory report then
//! lists what it allocates: any array other than its result breaks
//! `require_fused` (an intermediate array); any array, copy or node of a
//! recursive type breaks `require_no_alloc`, in the function and in the
//! recursive functions it calls.

use crate::report::{entries_of, Entry};
use crate::*;
use isc_syntax::ast::Modifier;
use isc_syntax::Diagnostic;
use isc_types::tast::TProgram;
use isc_types::types::Ty;

type R<T> = std::result::Result<T, Diagnostic>;

/// What the function allocates, beyond its result.
fn allocations(p: &mut Program, with_callees: bool) -> R<Vec<Entry>> {
    crate::passes::optimize_body(&mut p.body, &mut p.vars);
    for f in p.fns.iter_mut() {
        crate::passes::optimize_body(&mut f.body, &mut p.vars);
    }
    let mut reasons = Vec::new();
    crate::passes::ownership_body(&mut p.body, &p.vars, None, &mut reasons)?;
    for f in p.fns.iter_mut() {
        crate::passes::ownership_body(&mut f.body, &p.vars, None, &mut reasons)?;
    }
    p.copy_reasons = reasons;
    Ok(entries_of(p, with_callees)
        .into_iter()
        .filter(|e| match e.kind {
            "alloc" => e.why != "a result",
            "copy" => !e.why.ends_with("the destination of the caller"),
            // a new node allocates; one owner more does not
            "refcount" => !e.why.starts_with("one owner more"),
            _ => false,
        })
        .collect())
}

pub fn check(t: &TProgram, required: &[(usize, Vec<Ty>)]) -> R<()> {
    for (id, type_args) in required {
        let f = &t.fns[*id];
        for (m, at) in &f.requires {
            let (name, with_callees) = match m {
                Modifier::RequireFused => ("require_fused", false),
                Modifier::RequireNoAlloc => ("require_no_alloc", true),
                _ => continue,
            };
            let mut p = crate::lower::probe(t, *id, type_args.clone())?;
            let found: Vec<Entry> = allocations(&mut p, with_callees)?
                .into_iter()
                .filter(|e| *m == Modifier::RequireNoAlloc || e.kind != "refcount")
                .collect();
            if found.is_empty() {
                continue;
            }
            let mut d = Diagnostic::new(*at, format!("{} not satisfied (line {})", name, f.span.line));
            d.notes = found.iter().map(|e| format!("line {}: {} {}", e.line, e.what, e.why)).collect();
            return Err(d);
        }
    }
    Ok(())
}
