//! Minimal pretty printer for kernel terms. For display only.

use margin_canon::{Arena, Decl, ExprId, ExprNode, Level, Name};

const MAX_DEPTH: usize = 200;

fn level(l: &Level) -> String {
    fn succs(l: &Level, n: u64) -> String {
        match l {
            Level::Succ(a) => succs(a, n + 1),
            Level::Zero => n.to_string(),
            other if n == 0 => level(other),
            other => format!("{}+{n}", level(other)),
        }
    }
    match l {
        Level::Zero | Level::Succ(_) => succs(l, 0),
        Level::Max(a, b) => format!("max {} {}", level(a), level(b)),
        Level::IMax(a, b) => format!("imax {} {}", level(a), level(b)),
        Level::Param(n) => n.to_string(),
    }
}

/// Does `e` mention bound variable `k`?
fn has_bvar(a: &Arena, e: ExprId, k: u64, depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    match a.get(e) {
        ExprNode::BVar(i) => *i == k,
        ExprNode::App(f, x) => has_bvar(a, *f, k, depth + 1) || has_bvar(a, *x, k, depth + 1),
        ExprNode::Lam(t, b) | ExprNode::Pi(t, b) => {
            has_bvar(a, *t, k, depth + 1) || has_bvar(a, *b, k + 1, depth + 1)
        }
        ExprNode::Let(t, v, b, _) => {
            has_bvar(a, *t, k, depth + 1) || has_bvar(a, *v, k, depth + 1) || has_bvar(a, *b, k + 1, depth + 1)
        }
        ExprNode::Proj(_, _, x) => has_bvar(a, *x, k, depth + 1),
        _ => false,
    }
}

/// CE erases binder names, so the printer invents them by depth: x0, x1, ...
fn fresh(ctx: &[Name]) -> Name {
    Name::from_dotted(&format!("x{}", ctx.len()))
}

fn expr(a: &Arena, e: ExprId, ctx: &mut Vec<Name>, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "…".into();
    }
    let d = depth + 1;
    match a.get(e) {
        ExprNode::BVar(i) => {
            let i = *i as usize;
            if i < ctx.len() {
                ctx[ctx.len() - 1 - i].to_string()
            } else {
                format!("#{i}")
            }
        }
        ExprNode::Sort(l) => match a.level(*l) {
            Level::Zero => "Prop".into(),
            Level::Succ(x) if *x == Level::Zero => "Type".into(),
            l => format!("Sort ({})", level(&l)),
        },
        ExprNode::Const(n, ls) if a.levels(*ls).is_empty() => a.name(*n).to_string(),
        ExprNode::Const(n, ls) => {
            let ls: Vec<String> = a.levels(*ls).iter().map(|l| level(&a.level(*l))).collect();
            format!("{}.{{{}}}", a.name(*n), ls.join(", "))
        }
        ExprNode::App(..) => {
            let mut args = Vec::new();
            let mut head = e;
            while let ExprNode::App(f, x) = a.get(head) {
                args.push(*x);
                head = *f;
            }
            let mut s = atom(a, head, ctx, d);
            for x in args.iter().rev() {
                s.push(' ');
                s.push_str(&atom(a, *x, ctx, d));
            }
            s
        }
        ExprNode::Pi(t, b) => {
            let ts = expr(a, *t, ctx, d);
            let n = fresh(ctx);
            ctx.push(n.clone());
            let bs = expr(a, *b, ctx, d);
            ctx.pop();
            if has_bvar(a, *b, 0, 0) {
                format!("∀ ({n} : {ts}), {bs}")
            } else {
                format!("{} → {bs}", paren_if_arrow(a, *t, ts))
            }
        }
        ExprNode::Lam(t, b) => {
            let ts = expr(a, *t, ctx, d);
            let n = fresh(ctx);
            ctx.push(n.clone());
            let bs = expr(a, *b, ctx, d);
            ctx.pop();
            format!("fun ({n} : {ts}) => {bs}")
        }
        ExprNode::Let(t, v, b, _) => {
            let ts = expr(a, *t, ctx, d);
            let vs = expr(a, *v, ctx, d);
            let n = fresh(ctx);
            ctx.push(n.clone());
            let bs = expr(a, *b, ctx, d);
            ctx.pop();
            format!("let {n} : {ts} := {vs}; {bs}")
        }
        ExprNode::NatLit(bytes) => {
            if bytes.len() <= 16 {
                let mut v: u128 = 0;
                for b in bytes.iter() {
                    v = (v << 8) | *b as u128;
                }
                v.to_string()
            } else {
                format!("0x{}", hex::encode(bytes))
            }
        }
        ExprNode::StrLit(s) => format!("{s:?}"),
        ExprNode::Proj(_, i, x) => format!("{}.{}", atom(a, *x, ctx, d), i + 1),
    }
}

fn paren_if_arrow(a: &Arena, t: ExprId, s: String) -> String {
    if matches!(a.get(t), ExprNode::Pi(..)) {
        format!("({s})")
    } else {
        s
    }
}

fn atom(a: &Arena, e: ExprId, ctx: &mut Vec<Name>, depth: usize) -> String {
    let s = expr(a, e, ctx, depth);
    match a.get(e) {
        ExprNode::App(..) | ExprNode::Pi(..) | ExprNode::Lam(..) | ExprNode::Let(..) => format!("({s})"),
        _ => s,
    }
}

fn univs(ls: &[Name]) -> String {
    if ls.is_empty() {
        String::new()
    } else {
        let v: Vec<String> = ls.iter().map(|n| n.to_string()).collect();
        format!(".{{{}}}", v.join(", "))
    }
}

/// One-line rendering of a declaration.
pub fn render_decl(a: &Arena, d: &Decl) -> String {
    let mut ctx = Vec::new();
    match d {
        Decl::Statement { name, lparams, ty } => {
            format!("theorem {name}{} : {}", univs(lparams), expr(a, *ty, &mut ctx, 0))
        }
        Decl::Theorem { name, lparams, ty, .. } => {
            format!("theorem {name}{} : {} := <hidden>", univs(lparams), expr(a, *ty, &mut ctx, 0))
        }
        Decl::Definition { name, lparams, ty, val, .. } => format!(
            "def {name}{} : {} := {}",
            univs(lparams),
            expr(a, *ty, &mut ctx, 0),
            expr(a, *val, &mut ctx, 0)
        ),
        Decl::Opaque { name, lparams, ty, .. } => {
            format!("opaque {name}{} : {}", univs(lparams), expr(a, *ty, &mut ctx, 0))
        }
        Decl::Axiom { name, lparams, ty, .. } => {
            format!("axiom {name}{} : {}", univs(lparams), expr(a, *ty, &mut ctx, 0))
        }
        Decl::Quot { name, lparams, ty, .. } => {
            format!("quot {name}{} : {}", univs(lparams), expr(a, *ty, &mut ctx, 0))
        }
        Decl::Inductive(b) => {
            let mut parts = Vec::new();
            for t in &b.types {
                let ctors: Vec<String> = b
                    .ctors
                    .iter()
                    .filter(|c| c.induct == t.name)
                    .map(|c| format!("| {} : {}", c.name, expr(a, c.ty, &mut Vec::new(), 0)))
                    .collect();
                parts.push(format!(
                    "inductive {}{} : {} {}",
                    t.name,
                    univs(&t.lparams),
                    expr(a, t.ty, &mut Vec::new(), 0),
                    ctors.join(" ")
                ));
            }
            parts.join("; ")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_forall_and_arrow() {
        let mut a = Arena::new();
        let nat = a.cnst("Nat", vec![]);
        let eq = a.cnst("Eq", vec![Level::Succ(Box::new(Level::Zero))]);
        let b0 = a.bvar(0);
        let e1 = a.app(eq, nat);
        let e2 = a.app(e1, b0);
        let e3 = a.app(e2, b0);
        let ty = a.pi("n", nat, e3);
        let d = Decl::Statement { name: Name::from_dotted("refl"), lparams: vec![], ty };
        assert_eq!(render_decl(&a, &d), "theorem refl : ∀ (x0 : Nat), Eq.{1} Nat x0 x0");
        let arrow = a.pi("x", nat, nat);
        let d = Decl::Statement { name: Name::from_dotted("f"), lparams: vec![], ty: arrow };
        assert_eq!(render_decl(&a, &d), "theorem f : Nat → Nat");
    }
}
