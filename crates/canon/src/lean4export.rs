//! Conversion between CE declarations and the lean4export NDJSON format (version 3.1).
//!
//! - [`write_ndjson`] (no_std): CE → export text. The guest uses it to feed kernels that
//!   read lean4export files (nanoda).
//! - [`read_ndjson`] (feature `lean4export`): export text → CE. Used to import an
//!   environment or a user project exported by `lean4export`.

use crate::types::*;
use alloc::{format, string::String, vec::Vec};
use hashbrown::HashMap;
use core::fmt::Write;

pub const FORMAT_VERSION: &str = "3.1.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportError(pub String);

impl core::fmt::Display for ExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, ExportError> {
    Err(ExportError(msg.into()))
}

/// JSON string literal with escaping.
fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Big-endian magnitude → decimal string.
pub fn nat_to_decimal(be: &[u8]) -> String {
    let mut digits: Vec<u8> = Vec::new(); // little-endian base 10
    for &byte in be {
        // digits = digits * 256 + byte
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            let v = (*d as u32) * 256 + carry;
            *d = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    if digits.is_empty() {
        return "0".into();
    }
    digits.iter().rev().map(|d| (b'0' + d) as char).collect()
}

/// Decimal string → minimal big-endian magnitude.
pub fn decimal_to_nat(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    let mut le: Vec<u8> = Vec::new(); // little-endian base 256
    for b in s.bytes() {
        let mut carry = (b - b'0') as u32;
        for x in le.iter_mut() {
            let v = (*x as u32) * 10 + carry;
            *x = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            le.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    le.reverse();
    Some(le)
}

/// One export record, with lean4export indices resolved. Strings borrow from the arena
/// or the declarations.
#[derive(Clone, Debug)]
pub enum ExprRec<'a> {
    BVar(u16),
    Sort(u32),
    Const { name: u32, us: Vec<u32> },
    App { f: u32, a: u32 },
    Lam { name: u32, ty: u32, body: u32, bi: BinderInfo },
    Pi { name: u32, ty: u32, body: u32, bi: BinderInfo },
    Let { name: u32, ty: u32, val: u32, body: u32, nondep: bool },
    /// Big-endian magnitude.
    NatLit(&'a [u8]),
    StrLit(&'a str),
    Proj { type_name: u32, idx: u64, structure: u32 },
}

#[derive(Clone, Debug)]
pub struct IndRec {
    pub name: u32,
    pub lparams: Vec<u32>,
    pub ty: u32,
    pub all: Vec<u32>,
    pub ctors: Vec<u32>,
    pub is_rec: bool,
    pub is_reflexive: bool,
    pub is_unsafe: bool,
    pub num_params: u64,
    pub num_indices: u64,
    pub num_nested: u64,
}

#[derive(Clone, Debug)]
pub struct CtorRec {
    pub name: u32,
    pub lparams: Vec<u32>,
    pub ty: u32,
    pub is_unsafe: bool,
    pub induct: u32,
    pub cidx: u64,
    pub num_params: u64,
    pub num_fields: u64,
}

#[derive(Clone, Debug)]
pub struct RuleRec {
    pub ctor: u32,
    pub nfields: u64,
    pub rhs: u32,
}

#[derive(Clone, Debug)]
pub struct RecRec {
    pub name: u32,
    pub lparams: Vec<u32>,
    pub ty: u32,
    pub is_unsafe: bool,
    pub all: Vec<u32>,
    pub num_params: u64,
    pub num_indices: u64,
    pub num_motives: u64,
    pub num_minors: u64,
    pub rules: Vec<RuleRec>,
    pub k: bool,
}

#[derive(Clone, Debug)]
pub enum Record<'a> {
    NameStr { idx: u32, pre: u32, s: &'a str },
    NameNum { idx: u32, pre: u32, i: u32 },
    Succ { idx: u32, l: u32 },
    Max { idx: u32, a: u32, b: u32 },
    IMax { idx: u32, a: u32, b: u32 },
    Param { idx: u32, name: u32 },
    Expr { idx: u32, e: ExprRec<'a> },
    Axiom { name: u32, lparams: Vec<u32>, ty: u32, is_unsafe: bool },
    Thm { name: u32, lparams: Vec<u32>, ty: u32, val: u32 },
    Def { name: u32, lparams: Vec<u32>, ty: u32, val: u32, hints: Hints, safety: Safety },
    Opaque { name: u32, lparams: Vec<u32>, ty: u32, val: u32, is_unsafe: bool },
    Quot { name: u32, lparams: Vec<u32>, ty: u32, kind: QuotKind },
    Inductive { types: Vec<IndRec>, ctors: Vec<CtorRec>, recs: Vec<RecRec> },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum LevelKey {
    Succ(u32),
    Max(u32, u32),
    IMax(u32, u32),
    Param(u32),
}

type Sink<'s, 'a> = &'s mut dyn FnMut(Record<'a>) -> Result<(), ExportError>;

struct Emitter<'s, 'a> {
    arena: &'a Arena,
    sink: Sink<'s, 'a>,
    /// (prefix index, component) → index. The anonymous name is index 0.
    names: HashMap<(u32, &'a NamePart), u32, BuildFx>,
    n_names: u32,
    /// Structural level key (in export indices) → index. Zero is index 0.
    levels: HashMap<LevelKey, u32, BuildFx>,
    n_levels: u32,
    /// LevelId → index (`u32::MAX` = not emitted yet).
    level_cache: Vec<u32>,
    /// ExprId → index (`u32::MAX` = not emitted yet).
    exprs: Vec<u32>,
    n_exprs: u32,
}

#[derive(Default, Clone, Copy)]
struct BuildFx;
impl core::hash::BuildHasher for BuildFx {
    type Hasher = crate::fxhash::FxHasher;
    fn build_hasher(&self) -> Self::Hasher {
        crate::fxhash::FxHasher::default()
    }
}

impl<'s, 'a> Emitter<'s, 'a> {
    fn name(&mut self, n: &'a Name) -> Result<u32, ExportError> {
        let mut pre = 0u32;
        for part in &n.0 {
            if let Some(&k) = self.names.get(&(pre, part)) {
                pre = k;
                continue;
            }
            let idx = self.n_names;
            self.n_names += 1;
            match part {
                NamePart::Str(s) => (self.sink)(Record::NameStr { idx, pre, s })?,
                NamePart::Num(v) => {
                    let i = u32::try_from(*v).or_else(|_| err("numeric name component exceeds u32"))?;
                    (self.sink)(Record::NameNum { idx, pre, i })?
                }
            }
            self.names.insert((pre, part), idx);
            pre = idx;
        }
        Ok(pre)
    }

    fn level_key(&mut self, key: LevelKey) -> Result<u32, ExportError> {
        if let Some(&k) = self.levels.get(&key) {
            return Ok(k);
        }
        let idx = self.n_levels;
        self.n_levels += 1;
        (self.sink)(match key {
            LevelKey::Succ(a) => Record::Succ { idx, l: a },
            LevelKey::Max(a, b) => Record::Max { idx, a, b },
            LevelKey::IMax(a, b) => Record::IMax { idx, a, b },
            LevelKey::Param(n) => Record::Param { idx, name: n },
        })?;
        self.levels.insert(key, idx);
        Ok(idx)
    }

    fn level(&mut self, l: LevelId) -> Result<u32, ExportError> {
        let cached = self.level_cache[l.0 as usize];
        if cached != u32::MAX {
            return Ok(cached);
        }
        let arena: &'a Arena = self.arena;
        let idx = match arena.level_node(l) {
            LevelNode::Zero => 0,
            LevelNode::Succ(a) => {
                let a = self.level(a)?;
                self.level_key(LevelKey::Succ(a))?
            }
            LevelNode::Max(a, b) => {
                let (a, b) = (self.level(a)?, self.level(b)?);
                self.level_key(LevelKey::Max(a, b))?
            }
            LevelNode::IMax(a, b) => {
                let (a, b) = (self.level(a)?, self.level(b)?);
                self.level_key(LevelKey::IMax(a, b))?
            }
            LevelNode::Param(n) => {
                let n = self.name(arena.name(n))?;
                self.level_key(LevelKey::Param(n))?
            }
        };
        self.level_cache[l.0 as usize] = idx;
        Ok(idx)
    }

    fn lparams(&mut self, ps: &'a [Name]) -> Result<Vec<u32>, ExportError> {
        // nanoda requires `Level::Param(n)` to exist for every universe parameter.
        let mut v = Vec::with_capacity(ps.len());
        for p in ps {
            let n = self.name(p)?;
            self.level_key(LevelKey::Param(n))?;
            v.push(n);
        }
        Ok(v)
    }

    fn names(&mut self, ns: &'a [Name]) -> Result<Vec<u32>, ExportError> {
        ns.iter().map(|n| self.name(n)).collect()
    }

    fn ix(&self, e: ExprId) -> u32 {
        self.exprs[e.0 as usize]
    }

    fn expr(&mut self, root: ExprId) -> Result<u32, ExportError> {
        let arena: &'a Arena = self.arena;
        let mut stack = alloc::vec![(root, false)];
        while let Some((id, expanded)) = stack.pop() {
            if self.exprs[id.0 as usize] != u32::MAX {
                continue;
            }
            let node: &'a ExprNode = arena.get(id);
            if !expanded {
                stack.push((id, true));
                for c in node.children().as_slice().iter().copied().rev() {
                    stack.push((c, false));
                }
                continue;
            }
            let e = match node {
                ExprNode::BVar(i) => ExprRec::BVar(u16::try_from(*i).or_else(|_| err("bound variable index exceeds u16"))?),
                ExprNode::Sort(l) => ExprRec::Sort(self.level(*l)?),
                ExprNode::Const(n, ls) => {
                    let name = self.name(arena.name(*n))?;
                    let us = arena.levels(*ls).iter().map(|l| self.level(*l)).collect::<Result<Vec<_>, _>>()?;
                    ExprRec::Const { name, us }
                }
                ExprNode::App(f, a) => ExprRec::App { f: self.ix(*f), a: self.ix(*a) },
                // Binder names are erased: anonymous name (index 0), default binder info.
                ExprNode::Lam(t, b) => ExprRec::Lam { name: 0, ty: self.ix(*t), body: self.ix(*b), bi: BinderInfo::Default },
                ExprNode::Pi(t, b) => ExprRec::Pi { name: 0, ty: self.ix(*t), body: self.ix(*b), bi: BinderInfo::Default },
                ExprNode::Let(t, v, b, nondep) => ExprRec::Let {
                    name: 0,
                    ty: self.ix(*t),
                    val: self.ix(*v),
                    body: self.ix(*b),
                    nondep: *nondep,
                },
                ExprNode::NatLit(bytes) => ExprRec::NatLit(bytes),
                ExprNode::StrLit(s) => ExprRec::StrLit(s),
                ExprNode::Proj(n, i, x) => ExprRec::Proj { type_name: self.name(arena.name(*n))?, idx: *i, structure: self.ix(*x) },
            };
            let idx = self.n_exprs;
            self.n_exprs += 1;
            (self.sink)(Record::Expr { idx, e })?;
            self.exprs[id.0 as usize] = idx;
        }
        Ok(self.ix(root))
    }

    fn decl(&mut self, d: &'a Decl) -> Result<(), ExportError> {
        // Emit all expressions first, so declaration records only refer backwards.
        for e in d.exprs() {
            self.expr(e)?;
        }
        let rec = match d {
            Decl::Axiom { name, lparams, ty, is_unsafe } => Record::Axiom {
                name: self.name(name)?,
                lparams: self.lparams(lparams)?,
                ty: self.ix(*ty),
                is_unsafe: *is_unsafe,
            },
            Decl::Theorem { name, lparams, ty, val } => Record::Thm {
                name: self.name(name)?,
                lparams: self.lparams(lparams)?,
                ty: self.ix(*ty),
                val: self.ix(*val),
            },
            Decl::Definition { name, lparams, ty, val, hints, safety } => Record::Def {
                name: self.name(name)?,
                lparams: self.lparams(lparams)?,
                ty: self.ix(*ty),
                val: self.ix(*val),
                hints: *hints,
                safety: *safety,
            },
            Decl::Opaque { name, lparams, ty, val, is_unsafe } => Record::Opaque {
                name: self.name(name)?,
                lparams: self.lparams(lparams)?,
                ty: self.ix(*ty),
                val: self.ix(*val),
                is_unsafe: *is_unsafe,
            },
            Decl::Quot { name, lparams, ty, kind } => Record::Quot {
                name: self.name(name)?,
                lparams: self.lparams(lparams)?,
                ty: self.ix(*ty),
                kind: *kind,
            },
            Decl::Inductive(b) => {
                let mut types = Vec::with_capacity(b.types.len());
                for t in &b.types {
                    types.push(IndRec {
                        name: self.name(&t.name)?,
                        lparams: self.lparams(&t.lparams)?,
                        ty: self.ix(t.ty),
                        all: self.names(&t.all)?,
                        ctors: self.names(&t.ctors)?,
                        is_rec: t.is_rec,
                        is_reflexive: t.is_reflexive,
                        is_unsafe: t.is_unsafe,
                        num_params: t.num_params,
                        num_indices: t.num_indices,
                        num_nested: t.num_nested,
                    });
                }
                let mut ctors = Vec::with_capacity(b.ctors.len());
                for c in &b.ctors {
                    ctors.push(CtorRec {
                        name: self.name(&c.name)?,
                        lparams: self.lparams(&c.lparams)?,
                        ty: self.ix(c.ty),
                        is_unsafe: c.is_unsafe,
                        induct: self.name(&c.induct)?,
                        cidx: c.cidx,
                        num_params: c.num_params,
                        num_fields: c.num_fields,
                    });
                }
                let mut recs = Vec::with_capacity(b.recs.len());
                for r in &b.recs {
                    let mut rules = Vec::with_capacity(r.rules.len());
                    for x in &r.rules {
                        rules.push(RuleRec { ctor: self.name(&x.ctor)?, nfields: x.nfields, rhs: self.ix(x.rhs) });
                    }
                    recs.push(RecRec {
                        name: self.name(&r.name)?,
                        lparams: self.lparams(&r.lparams)?,
                        ty: self.ix(r.ty),
                        is_unsafe: r.is_unsafe,
                        all: self.names(&r.all)?,
                        num_params: r.num_params,
                        num_indices: r.num_indices,
                        num_motives: r.num_motives,
                        num_minors: r.num_minors,
                        rules,
                        k: r.k,
                    });
                }
                Record::Inductive { types, ctors, recs }
            }
            Decl::Statement { .. } => return err("statements cannot be exported"),
        };
        (self.sink)(rec)
    }
}

/// Emit declarations, in order, as lean4export records (without the meta record).
/// Names, levels and expressions come before the records that use them.
pub fn export_records<'a>(
    arena: &'a Arena,
    decls: &[&'a Decl],
    sink: &mut dyn FnMut(Record<'a>) -> Result<(), ExportError>,
) -> Result<(), ExportError> {
    let mut e = Emitter {
        arena,
        sink,
        names: HashMap::with_hasher(BuildFx),
        n_names: 1,
        levels: HashMap::with_hasher(BuildFx),
        n_levels: 1,
        level_cache: alloc::vec![u32::MAX; arena.level_count()],
        exprs: alloc::vec![u32::MAX; arena.len()],
        n_exprs: 0,
    };
    for d in decls {
        e.decl(d)?;
    }
    Ok(())
}

fn join(v: &[u32]) -> String {
    let mut s = String::new();
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{x}");
    }
    s
}

fn binder(bi: BinderInfo) -> &'static str {
    match bi {
        BinderInfo::Default => "default",
        BinderInfo::Implicit => "implicit",
        BinderInfo::StrictImplicit => "strictImplicit",
        BinderInfo::InstImplicit => "instImplicit",
    }
}

fn json_record(out: &mut String, r: &Record) -> Result<(), ExportError> {
    let lp = |v: &Vec<u32>| join(v);
    match r {
        Record::NameStr { idx, pre, s } => {
            let _ = write!(out, "{{\"in\":{idx},\"str\":{{\"pre\":{pre},\"str\":");
            json_str(out, s);
            out.push_str("}}");
        }
        Record::NameNum { idx, pre, i } => {
            let _ = write!(out, "{{\"in\":{idx},\"num\":{{\"pre\":{pre},\"i\":{i}}}}}");
        }
        Record::Succ { idx, l } => {
            let _ = write!(out, "{{\"il\":{idx},\"succ\":{l}}}");
        }
        Record::Max { idx, a, b } => {
            let _ = write!(out, "{{\"il\":{idx},\"max\":[{a},{b}]}}");
        }
        Record::IMax { idx, a, b } => {
            let _ = write!(out, "{{\"il\":{idx},\"imax\":[{a},{b}]}}");
        }
        Record::Param { idx, name } => {
            let _ = write!(out, "{{\"il\":{idx},\"param\":{name}}}");
        }
        Record::Expr { idx, e } => {
            let _ = write!(out, "{{\"ie\":{idx},");
            match e {
                ExprRec::BVar(i) => {
                    let _ = write!(out, "\"bvar\":{i}");
                }
                ExprRec::Sort(l) => {
                    let _ = write!(out, "\"sort\":{l}");
                }
                ExprRec::Const { name, us } => {
                    let _ = write!(out, "\"const\":{{\"name\":{name},\"us\":[{}]}}", join(us));
                }
                ExprRec::App { f, a } => {
                    let _ = write!(out, "\"app\":{{\"fn\":{f},\"arg\":{a}}}");
                }
                ExprRec::Lam { name, ty, body, bi } | ExprRec::Pi { name, ty, body, bi } => {
                    let tag = if matches!(e, ExprRec::Lam { .. }) { "lam" } else { "forallE" };
                    let _ = write!(
                        out,
                        "\"{tag}\":{{\"name\":{name},\"type\":{ty},\"body\":{body},\"binderInfo\":\"{}\"}}",
                        binder(*bi)
                    );
                }
                ExprRec::Let { name, ty, val, body, nondep } => {
                    let _ = write!(
                        out,
                        "\"letE\":{{\"name\":{name},\"type\":{ty},\"value\":{val},\"body\":{body},\"nondep\":{nondep}}}"
                    );
                }
                ExprRec::NatLit(b) => {
                    let _ = write!(out, "\"natVal\":\"{}\"", nat_to_decimal(b));
                }
                ExprRec::StrLit(s) => {
                    out.push_str("\"strVal\":");
                    json_str(out, s);
                }
                ExprRec::Proj { type_name, idx, structure } => {
                    let _ = write!(out, "\"proj\":{{\"typeName\":{type_name},\"idx\":{idx},\"struct\":{structure}}}");
                }
            }
            out.push('}');
        }
        Record::Axiom { name, lparams, ty, is_unsafe } => {
            let _ = write!(
                out,
                "{{\"axiom\":{{\"name\":{name},\"levelParams\":[{}],\"type\":{ty},\"isUnsafe\":{is_unsafe}}}}}",
                lp(lparams)
            );
        }
        Record::Thm { name, lparams, ty, val } => {
            let _ = write!(
                out,
                "{{\"thm\":{{\"name\":{name},\"levelParams\":[{}],\"type\":{ty},\"value\":{val}}}}}",
                lp(lparams)
            );
        }
        Record::Def { name, lparams, ty, val, hints, safety } => {
            let hints = match hints {
                Hints::Opaque => "\"opaque\"".into(),
                Hints::Abbrev => "\"abbrev\"".into(),
                Hints::Regular(h) => {
                    let h = u16::try_from(*h).or_else(|_| err("reducibility height exceeds u16"))?;
                    format!("{{\"regular\":{h}}}")
                }
            };
            let safety = match safety {
                Safety::Safe => "safe",
                Safety::Unsafe => "unsafe",
                Safety::Partial => "partial",
            };
            let _ = write!(
                out,
                "{{\"def\":{{\"name\":{name},\"levelParams\":[{}],\"type\":{ty},\"value\":{val},\"hints\":{hints},\"safety\":\"{safety}\"}}}}",
                lp(lparams)
            );
        }
        Record::Opaque { name, lparams, ty, val, is_unsafe } => {
            let _ = write!(
                out,
                "{{\"opaque\":{{\"name\":{name},\"levelParams\":[{}],\"type\":{ty},\"value\":{val},\"isUnsafe\":{is_unsafe}}}}}",
                lp(lparams)
            );
        }
        Record::Quot { name, lparams, ty, kind } => {
            let kind = match kind {
                QuotKind::Type => "type",
                QuotKind::Ctor => "ctor",
                QuotKind::Lift => "lift",
                QuotKind::Ind => "ind",
            };
            let _ = write!(
                out,
                "{{\"quot\":{{\"name\":{name},\"levelParams\":[{}],\"type\":{ty},\"kind\":\"{kind}\"}}}}",
                lp(lparams)
            );
        }
        Record::Inductive { types, ctors, recs } => {
            out.push_str("{\"inductive\":{\"types\":[");
            for (i, t) in types.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{{\"name\":{},\"levelParams\":[{}],\"type\":{},\"all\":[{}],\"ctors\":[{}],\"isRec\":{},\"isReflexive\":{},\"isUnsafe\":{},\"numParams\":{},\"numIndices\":{},\"numNested\":{}}}",
                    t.name, lp(&t.lparams), t.ty, join(&t.all), join(&t.ctors), t.is_rec, t.is_reflexive, t.is_unsafe,
                    t.num_params, t.num_indices, t.num_nested
                );
            }
            out.push_str("],\"ctors\":[");
            for (i, c) in ctors.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let _ = write!(
                    out,
                    "{{\"name\":{},\"levelParams\":[{}],\"type\":{},\"isUnsafe\":{},\"induct\":{},\"cidx\":{},\"numParams\":{},\"numFields\":{}}}",
                    c.name, lp(&c.lparams), c.ty, c.is_unsafe, c.induct, c.cidx, c.num_params, c.num_fields
                );
            }
            out.push_str("],\"recs\":[");
            for (i, r) in recs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                let rules: Vec<String> = r
                    .rules
                    .iter()
                    .map(|x| format!("{{\"ctor\":{},\"nfields\":{},\"rhs\":{}}}", x.ctor, x.nfields, x.rhs))
                    .collect();
                let _ = write!(
                    out,
                    "{{\"name\":{},\"levelParams\":[{}],\"type\":{},\"isUnsafe\":{},\"all\":[{}],\"numParams\":{},\"numIndices\":{},\"numMotives\":{},\"numMinors\":{},\"rules\":[{}],\"k\":{}}}",
                    r.name, lp(&r.lparams), r.ty, r.is_unsafe, join(&r.all), r.num_params, r.num_indices,
                    r.num_motives, r.num_minors, rules.join(","), r.k
                );
            }
            out.push_str("]}}");
        }
    }
    out.push('\n');
    Ok(())
}

/// Write declarations, in order, as a lean4export 3.1 file.
pub fn write_ndjson(arena: &Arena, decls: &[&Decl]) -> Result<String, ExportError> {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{{\"meta\":{{\"exporter\":{{\"name\":\"ce\",\"version\":\"1\"}},\"format\":{{\"version\":\"{FORMAT_VERSION}\"}},\"lean\":{{\"githash\":\"\",\"version\":\"\"}}}}}}"
    );
    export_records(arena, decls, &mut |r| json_record(&mut out, &r))?;
    Ok(out)
}

#[cfg(feature = "lean4export")]
pub use reader::{read_ndjson, read_ndjson_from};

#[cfg(feature = "lean4export")]
mod reader {
    use super::*;
    use alloc::vec;
    use serde_json::{Map, Value};

    struct R<'a> {
        arena: &'a mut Arena,
        names: Vec<NameId>,
        levels: Vec<LevelId>,
        exprs: Vec<ExprId>,
        line: usize,
    }

    fn u(v: &Value) -> Option<u64> {
        v.as_u64()
    }

    impl R<'_> {
        fn e<T>(&self, msg: &str) -> Result<T, ExportError> {
            err(format!("line {}: {msg}", self.line))
        }
        fn get<'v>(&self, o: &'v Map<String, Value>, k: &str) -> Result<&'v Value, ExportError> {
            o.get(k).map_or_else(|| self.e(&format!("missing field `{k}`")), Ok)
        }
        fn num(&self, o: &Map<String, Value>, k: &str) -> Result<u64, ExportError> {
            u(self.get(o, k)?).map_or_else(|| self.e(&format!("`{k}` is not a number")), Ok)
        }
        fn boolean(&self, o: &Map<String, Value>, k: &str) -> Result<bool, ExportError> {
            self.get(o, k)?.as_bool().map_or_else(|| self.e(&format!("`{k}` is not a bool")), Ok)
        }
        fn name_id_at(&self, i: u64) -> Result<NameId, ExportError> {
            self.names.get(i as usize).copied().map_or_else(|| self.e("bad name reference"), Ok)
        }
        fn name_id(&self, o: &Map<String, Value>, k: &str) -> Result<NameId, ExportError> {
            self.name_id_at(self.num(o, k)?)
        }
        fn name_at(&self, i: u64) -> Result<Name, ExportError> {
            Ok(self.arena.name(self.name_id_at(i)?).clone())
        }
        fn name(&self, o: &Map<String, Value>, k: &str) -> Result<Name, ExportError> {
            self.name_at(self.num(o, k)?)
        }
        fn level_at(&self, i: u64) -> Result<LevelId, ExportError> {
            self.levels.get(i as usize).copied().map_or_else(|| self.e("bad level reference"), Ok)
        }
        fn expr_at(&self, i: u64) -> Result<ExprId, ExportError> {
            self.exprs.get(i as usize).copied().map_or_else(|| self.e("bad expr reference"), Ok)
        }
        fn expr(&self, o: &Map<String, Value>, k: &str) -> Result<ExprId, ExportError> {
            self.expr_at(self.num(o, k)?)
        }
        fn names(&self, o: &Map<String, Value>, k: &str) -> Result<Vec<Name>, ExportError> {
            let arr = self.get(o, k)?.as_array().map_or_else(|| self.e("expected array"), Ok)?;
            arr.iter().map(|v| u(v).map_or_else(|| self.e("expected index"), |i| self.name_at(i))).collect()
        }
        fn obj<'v>(&self, v: &'v Value) -> Result<&'v Map<String, Value>, ExportError> {
            v.as_object().map_or_else(|| self.e("expected object"), Ok)
        }

        fn line(&mut self, o: &Map<String, Value>, out: &mut Vec<Decl>) -> Result<(), ExportError> {
            if o.contains_key("meta") {
                let v = o["meta"]["format"]["version"].as_str().unwrap_or("");
                if !v.starts_with("3.1.") {
                    return self.e(&format!("unsupported export format version `{v}`"));
                }
                return Ok(());
            }
            if let Some(k) = o.get("in") {
                if u(k) != Some(self.names.len() as u64) {
                    return self.e("name back-references must be continuous");
                }
                let n = if let Some(s) = o.get("str") {
                    let s = self.obj(s)?;
                    let mut p = self.name(s, "pre")?;
                    let st = self.get(s, "str")?.as_str().map_or_else(|| self.e("bad string"), Ok)?;
                    p.0.push(NamePart::Str(st.into()));
                    p
                } else if let Some(s) = o.get("num") {
                    let s = self.obj(s)?;
                    let mut p = self.name(s, "pre")?;
                    p.0.push(NamePart::Num(self.num(s, "i")?));
                    p
                } else {
                    return self.e("unknown name kind");
                };
                let id = self.arena.intern_name(&n);
                self.names.push(id);
                return Ok(());
            }
            if let Some(k) = o.get("il") {
                if u(k) != Some(self.levels.len() as u64) {
                    return self.e("level back-references must be continuous");
                }
                let l = if let Some(v) = o.get("succ") {
                    LevelNode::Succ(self.level_at(u(v).unwrap_or(u64::MAX))?)
                } else if let Some(v) = o.get("max").or(o.get("imax")) {
                    let a = v.get(0).and_then(u).unwrap_or(u64::MAX);
                    let b = v.get(1).and_then(u).unwrap_or(u64::MAX);
                    let (a, b) = (self.level_at(a)?, self.level_at(b)?);
                    if o.contains_key("max") { LevelNode::Max(a, b) } else { LevelNode::IMax(a, b) }
                } else if let Some(v) = o.get("param") {
                    LevelNode::Param(self.name_id_at(u(v).unwrap_or(u64::MAX))?)
                } else {
                    return self.e("unknown level kind");
                };
                let id = self.arena.mk_level(l);
                self.levels.push(id);
                return Ok(());
            }
            if let Some(k) = o.get("ie") {
                if u(k) != Some(self.exprs.len() as u64) {
                    return self.e("expr back-references must be continuous");
                }
                let id = self.expr_line(o)?;
                self.exprs.push(id);
                return Ok(());
            }
            out.push(self.decl_line(o)?);
            Ok(())
        }

        fn binder(&self, o: &Map<String, Value>) -> Result<BinderInfo, ExportError> {
            Ok(match self.get(o, "binderInfo")?.as_str() {
                Some("default") => BinderInfo::Default,
                Some("implicit") => BinderInfo::Implicit,
                Some("strictImplicit") => BinderInfo::StrictImplicit,
                Some("instImplicit") => BinderInfo::InstImplicit,
                _ => return self.e("bad binderInfo"),
            })
        }

        fn expr_line(&mut self, o: &Map<String, Value>) -> Result<ExprId, ExportError> {
            let node = if let Some(v) = o.get("bvar") {
                ExprNode::BVar(u(v).map_or_else(|| self.e("bad bvar"), Ok)?)
            } else if let Some(v) = o.get("sort") {
                ExprNode::Sort(self.level_at(u(v).unwrap_or(u64::MAX))?)
            } else if let Some(v) = o.get("const") {
                let c = self.obj(v)?;
                let us = self.get(c, "us")?.as_array().map_or_else(|| self.e("bad us"), Ok)?;
                let ls = us.iter().map(|x| self.level_at(u(x).unwrap_or(u64::MAX))).collect::<Result<Vec<_>, _>>()?;
                let name = self.name_id(c, "name")?;
                ExprNode::Const(name, self.arena.intern_levels(&ls))
            } else if let Some(v) = o.get("app") {
                let c = self.obj(v)?;
                ExprNode::App(self.expr(c, "fn")?, self.expr(c, "arg")?)
            } else if let Some(v) = o.get("lam").or(o.get("forallE")) {
                let c = self.obj(v)?;
                // Binder names and binder info are validated, then erased.
                self.name_id(c, "name")?;
                self.binder(c)?;
                let (t, b) = (self.expr(c, "type")?, self.expr(c, "body")?);
                if o.contains_key("lam") { ExprNode::Lam(t, b) } else { ExprNode::Pi(t, b) }
            } else if let Some(v) = o.get("letE") {
                let c = self.obj(v)?;
                self.name_id(c, "name")?;
                ExprNode::Let(
                    self.expr(c, "type")?,
                    self.expr(c, "value")?,
                    self.expr(c, "body")?,
                    self.boolean(c, "nondep")?,
                )
            } else if let Some(v) = o.get("natVal") {
                let s = v.as_str().map_or_else(|| self.e("natVal must be a string"), Ok)?;
                ExprNode::NatLit(decimal_to_nat(s).map_or_else(|| self.e("bad natVal"), Ok)?.into())
            } else if let Some(v) = o.get("strVal") {
                ExprNode::StrLit(v.as_str().map_or_else(|| self.e("bad strVal"), Ok)?.into())
            } else if let Some(v) = o.get("proj") {
                let c = self.obj(v)?;
                ExprNode::Proj(self.name_id(c, "typeName")?, self.num(c, "idx")?, self.expr(c, "struct")?)
            } else if let Some(v) = o.get("mdata") {
                // Metadata does not affect type checking: alias the inner expression.
                return self.expr(self.obj(v)?, "expr");
            } else {
                return self.e("unknown expression kind");
            };
            Ok(self.arena.mk(node))
        }

        fn decl_line(&mut self, o: &Map<String, Value>) -> Result<Decl, ExportError> {
            let (kind, v) = o.iter().next().map_or_else(|| self.e("empty line object"), Ok)?;
            if o.len() != 1 {
                return self.e("declaration line must have exactly one key");
            }
            let c = self.obj(v)?;
            let lp = |s: &Self| s.names(c, "levelParams");
            Ok(match kind.as_str() {
                "axiom" => Decl::Axiom {
                    name: self.name(c, "name")?,
                    lparams: lp(self)?,
                    ty: self.expr(c, "type")?,
                    is_unsafe: self.boolean(c, "isUnsafe")?,
                },
                "thm" => Decl::Theorem {
                    name: self.name(c, "name")?,
                    lparams: lp(self)?,
                    ty: self.expr(c, "type")?,
                    val: self.expr(c, "value")?,
                },
                "def" => Decl::Definition {
                    name: self.name(c, "name")?,
                    lparams: lp(self)?,
                    ty: self.expr(c, "type")?,
                    val: self.expr(c, "value")?,
                    hints: match self.get(c, "hints")? {
                        Value::String(s) if s == "opaque" => Hints::Opaque,
                        Value::String(s) if s == "abbrev" => Hints::Abbrev,
                        Value::Object(m) if m.contains_key("regular") => Hints::Regular(
                            u32::try_from(self.num(m, "regular")?).map_or_else(|_| self.e("bad height"), Ok)?,
                        ),
                        _ => return self.e("bad hints"),
                    },
                    safety: match self.get(c, "safety")?.as_str() {
                        Some("safe") => Safety::Safe,
                        Some("unsafe") => Safety::Unsafe,
                        Some("partial") => Safety::Partial,
                        _ => return self.e("bad safety"),
                    },
                },
                "opaque" => Decl::Opaque {
                    name: self.name(c, "name")?,
                    lparams: lp(self)?,
                    ty: self.expr(c, "type")?,
                    val: self.expr(c, "value")?,
                    is_unsafe: self.boolean(c, "isUnsafe")?,
                },
                "quot" => Decl::Quot {
                    name: self.name(c, "name")?,
                    lparams: lp(self)?,
                    ty: self.expr(c, "type")?,
                    kind: match self.get(c, "kind")?.as_str() {
                        Some("type") => QuotKind::Type,
                        Some("ctor") => QuotKind::Ctor,
                        Some("lift") => QuotKind::Lift,
                        Some("ind") => QuotKind::Ind,
                        _ => return self.e("bad quot kind"),
                    },
                },
                "inductive" => {
                    let arr = |s: &Self, k: &str| -> Result<Vec<Value>, ExportError> {
                        s.get(c, k)?.as_array().cloned().map_or_else(|| s.e("expected array"), Ok)
                    };
                    let mut types = vec![];
                    for t in arr(self, "types")? {
                        let t = self.obj(&t)?;
                        types.push(IndType {
                            name: self.name(t, "name")?,
                            lparams: self.names(t, "levelParams")?,
                            ty: self.expr(t, "type")?,
                            all: self.names(t, "all")?,
                            ctors: self.names(t, "ctors")?,
                            is_rec: self.boolean(t, "isRec")?,
                            is_reflexive: self.boolean(t, "isReflexive")?,
                            is_unsafe: self.boolean(t, "isUnsafe")?,
                            num_params: self.num(t, "numParams")?,
                            num_indices: self.num(t, "numIndices")?,
                            num_nested: self.num(t, "numNested")?,
                        });
                    }
                    let mut ctors = vec![];
                    for x in arr(self, "ctors")? {
                        let x = self.obj(&x)?;
                        ctors.push(CtorVal {
                            name: self.name(x, "name")?,
                            lparams: self.names(x, "levelParams")?,
                            ty: self.expr(x, "type")?,
                            is_unsafe: self.boolean(x, "isUnsafe")?,
                            induct: self.name(x, "induct")?,
                            cidx: self.num(x, "cidx")?,
                            num_params: self.num(x, "numParams")?,
                            num_fields: self.num(x, "numFields")?,
                        });
                    }
                    let mut recs = vec![];
                    for x in arr(self, "recs")? {
                        let x = self.obj(&x)?;
                        let mut rules = vec![];
                        for rv in self.get(x, "rules")?.as_array().map_or_else(|| self.e("bad rules"), Ok)? {
                            let rv = self.obj(rv)?;
                            rules.push(RecRule {
                                ctor: self.name(rv, "ctor")?,
                                nfields: self.num(rv, "nfields")?,
                                rhs: self.expr(rv, "rhs")?,
                            });
                        }
                        recs.push(RecVal {
                            name: self.name(x, "name")?,
                            lparams: self.names(x, "levelParams")?,
                            ty: self.expr(x, "type")?,
                            is_unsafe: self.boolean(x, "isUnsafe")?,
                            all: self.names(x, "all")?,
                            num_params: self.num(x, "numParams")?,
                            num_indices: self.num(x, "numIndices")?,
                            num_motives: self.num(x, "numMotives")?,
                            num_minors: self.num(x, "numMinors")?,
                            rules,
                            k: self.boolean(x, "k")?,
                        });
                    }
                    Decl::Inductive(InductiveBlock { types, ctors, recs })
                }
                other => return self.e(&format!("unknown declaration kind `{other}`")),
            })
        }
    }

    /// Read a lean4export 3.1 file. Returns the declarations in file order.
    pub fn read_ndjson(text: &str, arena: &mut Arena) -> Result<Vec<Decl>, ExportError> {
        read_ndjson_from(std::io::Cursor::new(text.as_bytes()), arena, &mut |_| {})
    }

    /// Streaming variant for large exports (Mathlib): reads line by line, never holds the
    /// whole file in memory. `progress` is called with the line count every 1M lines.
    pub fn read_ndjson_from<B: std::io::BufRead>(
        mut input: B,
        arena: &mut Arena,
        progress: &mut dyn FnMut(usize),
    ) -> Result<Vec<Decl>, ExportError> {
        let mut r = R { arena, names: vec![NameId(0)], levels: vec![LevelId(0)], exprs: vec![], line: 0 };
        let mut out = Vec::new();
        let mut line = String::new();
        loop {
            line.clear();
            let n = input.read_line(&mut line).map_err(|e| ExportError(format!("read error: {e}")))?;
            if n == 0 {
                break;
            }
            r.line += 1;
            if r.line.is_multiple_of(1_000_000) {
                progress(r.line);
            }
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&line).map_err(|e| ExportError(format!("line {}: {e}", r.line)))?;
            let Value::Object(o) = v else { return err(format!("line {}: expected object", r.line)) };
            r.line(&o, &mut out)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_roundtrip() {
        for s in ["0", "1", "255", "256", "65535", "18446744073709551616", "340282366920938463463374607431768211457"] {
            let b = decimal_to_nat(s).unwrap();
            assert!(b.first() != Some(&0));
            assert_eq!(nat_to_decimal(&b), s);
        }
        assert!(decimal_to_nat("012").is_none());
        assert!(decimal_to_nat("1a").is_none());
    }
}
