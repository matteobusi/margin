use crate::types::*;
use crate::wire::{Error, Reader, Writer};
use alloc::{collections::BTreeMap, vec::Vec};

const MAGIC: &[u8; 3] = b"CE1";

/// Build the local expression table: post-order DFS over `roots`, children left to
/// right, first occurrence wins. Iterative, so deep terms do not overflow the stack.
fn build_table(arena: &Arena, roots: &[ExprId]) -> (Vec<ExprId>, BTreeMap<ExprId, u64>) {
    let mut table = Vec::new();
    let mut local: BTreeMap<ExprId, u64> = BTreeMap::new();
    let mut stack: Vec<(ExprId, bool)> = Vec::new();
    for &root in roots {
        stack.push((root, false));
        while let Some((id, expanded)) = stack.pop() {
            if local.contains_key(&id) {
                continue;
            }
            if expanded {
                local.insert(id, table.len() as u64);
                table.push(id);
            } else {
                stack.push((id, true));
                for c in arena.get(id).children().into_iter().rev() {
                    if !local.contains_key(&c) {
                        stack.push((c, false));
                    }
                }
            }
        }
    }
    (table, local)
}

/// Encode a declaration to its canonical bytes.
pub fn encode_decl(arena: &Arena, decl: &Decl) -> Vec<u8> {
    let (table, local) = build_table(arena, &decl.exprs());
    let r = |id: &ExprId| local[id];
    let mut w = Writer::new();
    w.raw(MAGIC);
    w.uvar(table.len() as u64);
    for id in &table {
        match arena.get(*id) {
            ExprNode::BVar(i) => {
                w.u8(0x00);
                w.uvar(*i);
            }
            ExprNode::Sort(l) => {
                w.u8(0x01);
                write_level(&mut w, arena, *l);
            }
            ExprNode::Const(n, ls) => {
                w.u8(0x02);
                w.name(arena.name(*n));
                let ls = arena.levels(*ls);
                w.uvar(ls.len() as u64);
                for l in ls {
                    write_level(&mut w, arena, *l);
                }
            }
            ExprNode::App(f, a) => {
                w.u8(0x03);
                w.uvar(r(f));
                w.uvar(r(a));
            }
            // Binder names and binder info are erased: anonymous name, default binder.
            ExprNode::Lam(t, b) | ExprNode::Pi(t, b) => {
                let tag = if matches!(arena.get(*id), ExprNode::Lam(..)) { 0x04 } else { 0x05 };
                w.u8(tag);
                w.uvar(0);
                w.u8(0);
                w.uvar(r(t));
                w.uvar(r(b));
            }
            ExprNode::Let(t, v, b, nondep) => {
                w.u8(0x06);
                w.uvar(0);
                w.uvar(r(t));
                w.uvar(r(v));
                w.uvar(r(b));
                w.u8(*nondep as u8);
            }
            ExprNode::NatLit(bytes) => {
                w.u8(0x07);
                w.bytes(bytes);
            }
            ExprNode::StrLit(s) => {
                w.u8(0x08);
                w.str(s);
            }
            ExprNode::Proj(n, i, e) => {
                w.u8(0x09);
                w.name(arena.name(*n));
                w.uvar(*i);
                w.uvar(r(e));
            }
        }
    }
    let b = |w: &mut Writer, x: bool| w.u8(x as u8);
    match decl {
        Decl::Axiom { name, lparams, ty, is_unsafe } => {
            w.u8(0x00);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
            b(&mut w, *is_unsafe);
        }
        Decl::Definition { name, lparams, ty, val, hints, safety } => {
            w.u8(0x01);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
            w.uvar(r(val));
            match hints {
                Hints::Opaque => w.u8(0),
                Hints::Abbrev => w.u8(1),
                Hints::Regular(h) => {
                    w.u8(2);
                    w.uvar(*h as u64);
                }
            }
            w.u8(*safety as u8);
        }
        Decl::Theorem { name, lparams, ty, val } => {
            w.u8(0x02);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
            w.uvar(r(val));
        }
        Decl::Opaque { name, lparams, ty, val, is_unsafe } => {
            w.u8(0x03);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
            w.uvar(r(val));
            b(&mut w, *is_unsafe);
        }
        Decl::Quot { name, lparams, ty, kind } => {
            w.u8(0x04);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
            w.u8(*kind as u8);
        }
        Decl::Inductive(blk) => {
            w.u8(0x05);
            w.uvar(blk.types.len() as u64);
            for t in &blk.types {
                w.name(&t.name);
                w.names(&t.lparams);
                w.uvar(r(&t.ty));
                w.names(&t.all);
                w.names(&t.ctors);
                b(&mut w, t.is_rec);
                b(&mut w, t.is_reflexive);
                b(&mut w, t.is_unsafe);
                w.uvar(t.num_params);
                w.uvar(t.num_indices);
                w.uvar(t.num_nested);
            }
            w.uvar(blk.ctors.len() as u64);
            for c in &blk.ctors {
                w.name(&c.name);
                w.names(&c.lparams);
                w.uvar(r(&c.ty));
                b(&mut w, c.is_unsafe);
                w.name(&c.induct);
                w.uvar(c.cidx);
                w.uvar(c.num_params);
                w.uvar(c.num_fields);
            }
            w.uvar(blk.recs.len() as u64);
            for rv in &blk.recs {
                w.name(&rv.name);
                w.names(&rv.lparams);
                w.uvar(r(&rv.ty));
                b(&mut w, rv.is_unsafe);
                w.names(&rv.all);
                w.uvar(rv.num_params);
                w.uvar(rv.num_indices);
                w.uvar(rv.num_motives);
                w.uvar(rv.num_minors);
                w.uvar(rv.rules.len() as u64);
                for rule in &rv.rules {
                    w.name(&rule.ctor);
                    w.uvar(rule.nfields);
                    w.uvar(r(&rule.rhs));
                }
                b(&mut w, rv.k);
            }
        }
        Decl::Statement { name, lparams, ty } => {
            w.u8(0x06);
            w.name(name);
            w.names(lparams);
            w.uvar(r(ty));
        }
    }
    w.finish()
}

fn write_level(w: &mut Writer, arena: &Arena, l: LevelId) {
    match arena.level_node(l) {
        LevelNode::Zero => w.u8(0),
        LevelNode::Succ(a) => {
            w.u8(1);
            write_level(w, arena, a);
        }
        LevelNode::Max(a, b) => {
            w.u8(2);
            write_level(w, arena, a);
            write_level(w, arena, b);
        }
        LevelNode::IMax(a, b) => {
            w.u8(3);
            write_level(w, arena, a);
            write_level(w, arena, b);
        }
        LevelNode::Param(n) => {
            w.u8(4);
            w.name(arena.name(n));
        }
    }
}

/// Maximum nesting depth for levels.
const MAX_LEVEL_DEPTH: usize = 256;

fn read_level(r: &mut Reader, arena: &mut Arena, depth: usize) -> Result<LevelId, Error> {
    if depth > MAX_LEVEL_DEPTH {
        return Err(Error::TooDeep);
    }
    let node = match r.u8()? {
        0 => LevelNode::Zero,
        1 => LevelNode::Succ(read_level(r, arena, depth + 1)?),
        2 => LevelNode::Max(read_level(r, arena, depth + 1)?, read_level(r, arena, depth + 1)?),
        3 => LevelNode::IMax(read_level(r, arena, depth + 1)?, read_level(r, arena, depth + 1)?),
        4 => LevelNode::Param(arena.intern_name_raw(r.name_raw()?)?),
        t => return Err(Error::BadTag(t)),
    };
    Ok(arena.mk_level(node))
}

fn bool(r: &mut Reader) -> Result<bool, Error> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        t => Err(Error::BadTag(t)),
    }
}

/// Decode canonical bytes into `arena`. Rejects every non-canonical encoding.
pub fn decode_decl(bytes: &[u8], arena: &mut Arena) -> Result<Decl, Error> {
    let decl = decode_decl_trusted(bytes, arena)?;
    // Strong canonicity check: the bytes must be exactly what the encoder produces.
    // This rejects duplicate or unreferenced table nodes and any other ordering.
    if encode_decl(arena, &decl) != bytes {
        return Err(Error::NonCanonical);
    }
    Ok(decl)
}

/// Decode without the canonicity check (well-formedness is still checked).
///
/// Use only where canonicity does not matter for soundness: environment bytes are
/// pinned by the Merkle root, and user declarations are type-checked by the kernel.
/// Anything that is hashed or compared byte-wise must use [`decode_decl`].
pub fn decode_decl_trusted(bytes: &[u8], arena: &mut Arena) -> Result<Decl, Error> {
    let mut r = Reader::new(bytes);
    if r.raw(3)? != MAGIC {
        return Err(Error::BadMagic);
    }
    let n = r.count()?;
    let mut map: Vec<ExprId> = Vec::with_capacity(n);
    fn rf(r: &mut Reader, map: &[ExprId]) -> Result<ExprId, Error> {
        let i = r.uvar()?;
        usize::try_from(i).ok().and_then(|i| map.get(i).copied()).ok_or(Error::BadRef)
    }
    for _ in 0..n {
        let node = match r.u8()? {
            0x00 => ExprNode::BVar(r.uvar()?),
            0x01 => ExprNode::Sort(read_level(&mut r, arena, 0)?),
            0x02 => {
                let name = arena.intern_name_raw(r.name_raw()?)?;
                let k = r.count()?;
                let ls = (0..k).map(|_| read_level(&mut r, arena, 0)).collect::<Result<Vec<_>, _>>()?;
                ExprNode::Const(name, arena.intern_levels(&ls))
            }
            0x03 => ExprNode::App(rf(&mut r, &map)?, rf(&mut r, &map)?),
            tag @ (0x04 | 0x05) => {
                // Binder name and info are read and dropped (the canonical form has
                // the anonymous name and binder info 0; `decode_decl` enforces it).
                r.name_raw()?;
                BinderInfo::from_u8(r.u8()?).ok_or(Error::BadTag(0xff))?;
                let t = rf(&mut r, &map)?;
                let b = rf(&mut r, &map)?;
                if tag == 0x04 {
                    ExprNode::Lam(t, b)
                } else {
                    ExprNode::Pi(t, b)
                }
            }
            0x06 => {
                r.name_raw()?;
                let t = rf(&mut r, &map)?;
                let v = rf(&mut r, &map)?;
                let b = rf(&mut r, &map)?;
                ExprNode::Let(t, v, b, bool(&mut r)?)
            }
            0x07 => {
                let b = r.bytes()?;
                if b.first() == Some(&0) {
                    return Err(Error::NonCanonical);
                }
                ExprNode::NatLit(b.into())
            }
            0x08 => ExprNode::StrLit(r.str()?.into()),
            0x09 => {
                let name = arena.intern_name_raw(r.name_raw()?)?;
                let i = r.uvar()?;
                ExprNode::Proj(name, i, rf(&mut r, &map)?)
            }
            t => return Err(Error::BadTag(t)),
        };
        map.push(arena.mk(node));
    }
    let decl = match r.u8()? {
        0x00 => Decl::Axiom {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
            is_unsafe: bool(&mut r)?,
        },
        0x01 => Decl::Definition {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
            val: rf(&mut r, &map)?,
            hints: match r.u8()? {
                0 => Hints::Opaque,
                1 => Hints::Abbrev,
                2 => Hints::Regular(u32::try_from(r.uvar()?).map_err(|_| Error::Overflow)?),
                t => return Err(Error::BadTag(t)),
            },
            safety: match r.u8()? {
                0 => Safety::Safe,
                1 => Safety::Unsafe,
                2 => Safety::Partial,
                t => return Err(Error::BadTag(t)),
            },
        },
        0x02 => Decl::Theorem {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
            val: rf(&mut r, &map)?,
        },
        0x03 => Decl::Opaque {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
            val: rf(&mut r, &map)?,
            is_unsafe: bool(&mut r)?,
        },
        0x04 => Decl::Quot {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
            kind: match r.u8()? {
                0 => QuotKind::Type,
                1 => QuotKind::Ctor,
                2 => QuotKind::Lift,
                3 => QuotKind::Ind,
                t => return Err(Error::BadTag(t)),
            },
        },
        0x05 => {
            let n = r.count()?;
            let mut types = Vec::with_capacity(n);
            for _ in 0..n {
                types.push(IndType {
                    name: r.name()?,
                    lparams: r.names()?,
                    ty: rf(&mut r, &map)?,
                    all: r.names()?,
                    ctors: r.names()?,
                    is_rec: bool(&mut r)?,
                    is_reflexive: bool(&mut r)?,
                    is_unsafe: bool(&mut r)?,
                    num_params: r.uvar()?,
                    num_indices: r.uvar()?,
                    num_nested: r.uvar()?,
                });
            }
            let n = r.count()?;
            let mut ctors = Vec::with_capacity(n);
            for _ in 0..n {
                ctors.push(CtorVal {
                    name: r.name()?,
                    lparams: r.names()?,
                    ty: rf(&mut r, &map)?,
                    is_unsafe: bool(&mut r)?,
                    induct: r.name()?,
                    cidx: r.uvar()?,
                    num_params: r.uvar()?,
                    num_fields: r.uvar()?,
                });
            }
            let n = r.count()?;
            let mut recs = Vec::with_capacity(n);
            for _ in 0..n {
                let name = r.name()?;
                let lparams = r.names()?;
                let ty = rf(&mut r, &map)?;
                let is_unsafe = bool(&mut r)?;
                let all = r.names()?;
                let num_params = r.uvar()?;
                let num_indices = r.uvar()?;
                let num_motives = r.uvar()?;
                let num_minors = r.uvar()?;
                let nr = r.count()?;
                let mut rules = Vec::with_capacity(nr);
                for _ in 0..nr {
                    rules.push(RecRule { ctor: r.name()?, nfields: r.uvar()?, rhs: rf(&mut r, &map)? });
                }
                let k = bool(&mut r)?;
                recs.push(RecVal {
                    name, lparams, ty, is_unsafe, all, num_params, num_indices, num_motives, num_minors, rules, k,
                });
            }
            Decl::Inductive(InductiveBlock { types, ctors, recs })
        }
        0x06 => Decl::Statement {
            name: r.name()?,
            lparams: r.names()?,
            ty: rf(&mut r, &map)?,
        },
        t => return Err(Error::BadTag(t)),
    };
    r.finish()?;
    Ok(decl)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// `theorem id_eq (n : Nat) : n = n := fun n => Eq.refl n`
    fn sample(a: &mut Arena) -> Decl {
        let nat = a.cnst("Nat", vec![]);
        let one = Level::Succ(alloc::boxed::Box::new(Level::Zero));
        let eq = a.cnst("Eq", vec![one]);
        let refl = a.cnst("Eq.refl", vec![Level::Succ(alloc::boxed::Box::new(Level::Zero))]);
        let b0 = a.bvar(0);
        let eq_nat = a.app(eq, nat);
        let eq_n = a.app(eq_nat, b0);
        let eq_n_n = a.app(eq_n, b0);
        let ty = a.pi("n", nat, eq_n_n);
        let refl_nat = a.app(refl, nat);
        let body = a.app(refl_nat, b0);
        let val = a.lam("n", nat, body);
        Decl::Theorem { name: Name::from_dotted("id_eq"), lparams: vec![], ty, val }
    }

    #[test]
    fn roundtrip_same_bytes_in_fresh_arena() {
        let mut a = Arena::new();
        let d = sample(&mut a);
        let bytes = encode_decl(&a, &d);
        let mut b = Arena::new();
        let _noise = b.nat_lit(42); // different arena layout must not matter
        let d2 = decode_decl(&bytes, &mut b).unwrap();
        assert_eq!(encode_decl(&b, &d2), bytes);
    }

    #[test]
    fn sharing_is_encoded_once() {
        let mut a = Arena::new();
        let d = sample(&mut a);
        let bytes = encode_decl(&a, &d);
        // Nat, Eq, Eq.refl, bvar0, 3 apps for type, pi, 2 apps for body, lam = 11 nodes.
        assert_eq!(bytes[3], 11);
    }

    #[test]
    fn rejects_trailing_and_noncanonical() {
        let mut a = Arena::new();
        let d = sample(&mut a);
        let mut bytes = encode_decl(&a, &d);
        bytes.push(0);
        assert_eq!(decode_decl(&bytes, &mut Arena::new()), Err(Error::TrailingBytes));

        // A table with an extra unreferenced node is not canonical.
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.uvar(2);
        w.u8(0x00);
        w.uvar(0); // bvar 0
        w.u8(0x01);
        w.level(&Level::Zero); // Sort 0 (unreferenced)
        w.u8(0x00);
        w.name(&Name::from_dotted("ax"));
        w.names(&[]);
        w.uvar(0);
        w.u8(0);
        assert_eq!(decode_decl(&w.finish(), &mut Arena::new()), Err(Error::NonCanonical));
    }

    #[test]
    fn rejects_forward_reference() {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.uvar(1);
        w.u8(0x03);
        w.uvar(0);
        w.uvar(0); // app referring to itself
        assert_eq!(decode_decl(&w.finish(), &mut Arena::new()), Err(Error::BadRef));
    }
}
