//! Trusted environments: axiom sets, environment leaves, and openings for the prover.
//!
//! Each environment declaration is a Merkle leaf that commits to
//! - its name,
//! - its *header* (for a theorem: name, universe parameters and type, without the proof;
//!   for anything else: the full declaration),
//! - its full canonical encoding,
//! - the set of axioms it depends on, transitively (through proofs too).
//!
//! The prover can open a theorem by its header only. The kernel then sees it as an
//! opaque constant, and the axiom check uses the committed axiom set instead of
//! walking the proof. This keeps library proofs out of the zkVM.

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};
use margin_canon::merkle::{leaf_hash, EnvTree, MultiProof};
use margin_canon::wire::{Error, Reader, Writer};
use margin_canon::{axioms_hash, decl_hash, encode_decl, name_hash, Arena, Decl, ExprId, ExprNode, Hash32, Name};

/// How one environment declaration is revealed to the guest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opening {
    /// The full canonical bytes.
    Full(Vec<u8>),
    /// A theorem without its proof: CE bytes of its `Statement` form, plus the hash of
    /// the full declaration (needed to recompute the leaf).
    Header { header: Vec<u8>, full_hash: Hash32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvEntry {
    pub opening: Opening,
    /// Axioms the declaration depends on, transitively. Sorted, no duplicates.
    pub axioms: Vec<Name>,
}

impl EnvEntry {
    pub fn encode(&self, w: &mut Writer) {
        match &self.opening {
            Opening::Full(b) => {
                w.u8(0);
                w.bytes(b);
            }
            Opening::Header { header, full_hash } => {
                w.u8(1);
                w.bytes(header);
                w.raw(full_hash);
            }
        }
        w.names(&self.axioms);
    }

    pub fn decode(r: &mut Reader) -> Result<Self, Error> {
        let opening = match r.u8()? {
            0 => Opening::Full(r.bytes()?.to_vec()),
            1 => Opening::Header { header: r.bytes()?.to_vec(), full_hash: r.array32()? },
            t => return Err(Error::BadTag(t)),
        };
        Ok(Self { opening, axioms: r.names()? })
    }
}

/// The header form of a declaration: theorems lose their proof.
pub fn header_of(d: &Decl) -> Option<Decl> {
    match d {
        Decl::Theorem { name, lparams, ty, .. } => {
            Some(Decl::Statement { name: name.clone(), lparams: lparams.clone(), ty: *ty })
        }
        _ => None,
    }
}

/// Leaf hash of an environment declaration given its full CE bytes.
pub fn full_leaf(arena: &Arena, d: &Decl, full: &[u8], axioms: &[Name]) -> Hash32 {
    let full_hash = decl_hash(full);
    let header_hash = header_of(d).map_or(full_hash, |h| decl_hash(&encode_decl(arena, &h)));
    leaf_hash(&name_hash(&d.key()), &header_hash, &full_hash, &axioms_hash(axioms))
}

/// Constants referenced by the expressions reachable from `roots`.
pub fn constants(arena: &Arena, roots: &[ExprId]) -> alloc::collections::BTreeSet<Name> {
    let mut seen = alloc::collections::BTreeSet::new();
    let mut ids = alloc::collections::BTreeSet::new();
    let (mut nat, mut string) = (false, false);
    let mut stack: Vec<ExprId> = roots.to_vec();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = arena.get(id);
        match node {
            ExprNode::Const(n, _) | ExprNode::Proj(n, _, _) => {
                ids.insert(*n);
            }
            ExprNode::NatLit(_) => nat = true,
            ExprNode::StrLit(_) => string = true,
            _ => {}
        }
        stack.extend(node.children());
    }
    let mut out: alloc::collections::BTreeSet<Name> = ids.into_iter().map(|n| arena.name(n).clone()).collect();
    if nat {
        out.insert(Name::from_dotted("Nat"));
    }
    if string {
        out.insert(Name::from_dotted("String"));
    }
    out
}

/// A trusted environment with its Merkle tree.
pub struct Env {
    pub decls: Vec<Decl>,
    pub bytes: Vec<Vec<u8>>,
    /// Transitive axiom set of each declaration.
    pub axioms: Vec<Vec<Name>>,
    /// Constant name → declaration index.
    pub by_name: BTreeMap<Name, usize>,
    /// Declaration index → leaf index.
    pub leaf: Vec<usize>,
    pub tree: EnvTree,
}

impl Env {
    /// Build an environment: canonical bytes, transitive axiom sets, Merkle tree.
    ///
    /// Fails on duplicate names, references to unknown constants, or cycles between
    /// declarations (Lean environments are acyclic up to self-references).
    pub fn build(arena: &Arena, decls: Vec<Decl>) -> Result<Self, String> {
        let bytes: Vec<Vec<u8>> = decls.iter().map(|d| encode_decl(arena, d)).collect();
        let mut by_name = BTreeMap::new();
        for (i, d) in decls.iter().enumerate() {
            for n in d.defined_names() {
                if by_name.insert(n.clone(), i).is_some() {
                    return Err(alloc::format!("duplicate constant `{n}`"));
                }
            }
        }
        let deps: Vec<Vec<usize>> = decls
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let mut v: Vec<usize> = Vec::new();
                for c in constants(arena, &d.exprs()) {
                    match by_name.get(&c) {
                        Some(&j) if j != i => v.push(j),
                        Some(_) => {}
                        None => return Err(alloc::format!("`{}` refers to unknown constant `{c}`", d.key())),
                    }
                }
                v.sort_unstable();
                v.dedup();
                Ok(v)
            })
            .collect::<Result<_, String>>()?;

        // Iterative post-order DFS; 0 = new, 1 = on stack, 2 = done.
        let mut state = vec![0u8; decls.len()];
        let mut axioms: Vec<Vec<Name>> = vec![Vec::new(); decls.len()];
        for start in 0..decls.len() {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            state[start] = 1;
            while let Some((i, k)) = stack.pop() {
                if k < deps[i].len() {
                    stack.push((i, k + 1));
                    let j = deps[i][k];
                    match state[j] {
                        0 => {
                            state[j] = 1;
                            stack.push((j, 0));
                        }
                        1 => return Err(alloc::format!("cyclic dependency through `{}`", decls[j].key())),
                        _ => {}
                    }
                } else {
                    let mut set: Vec<Name> = deps[i].iter().flat_map(|&j| axioms[j].iter().cloned()).collect();
                    if let Decl::Axiom { name, .. } = &decls[i] {
                        set.push(name.clone());
                    }
                    set.sort();
                    set.dedup();
                    axioms[i] = set;
                    state[i] = 2;
                }
            }
        }

        Self::from_parts(arena, decls, bytes, axioms)
    }

    /// Rebuild an environment from declarations, their CE bytes and their (already
    /// computed) axiom sets, for example from a cache file. Recomputes every leaf, so the
    /// root reflects exactly the given data.
    pub fn from_parts(arena: &Arena, decls: Vec<Decl>, bytes: Vec<Vec<u8>>, axioms: Vec<Vec<Name>>) -> Result<Self, String> {
        if decls.len() != bytes.len() || decls.len() != axioms.len() {
            return Err("environment parts have different lengths".into());
        }
        let mut by_name = BTreeMap::new();
        for (i, d) in decls.iter().enumerate() {
            for n in d.defined_names() {
                if by_name.insert(n.clone(), i).is_some() {
                    return Err(alloc::format!("duplicate constant `{n}`"));
                }
            }
        }
        let keyed: Vec<(Hash32, Hash32)> = decls
            .iter()
            .enumerate()
            .map(|(i, d)| (name_hash(&d.key()), full_leaf(arena, d, &bytes[i], &axioms[i])))
            .collect();
        let tree = EnvTree::build(keyed.clone()).ok_or("duplicate declaration keys")?;
        let leaf = keyed.iter().map(|(k, _)| tree.find(k).expect("in tree")).collect();
        Ok(Self { decls, bytes, axioms, by_name, leaf, tree })
    }

    /// Openings for the given declarations (full, or header-only for theorems listed in
    /// `header_only`), in leaf order, with one multi-proof.
    pub fn open(&self, arena: &Arena, decls: &[usize], header_only: &[usize]) -> (Vec<EnvEntry>, MultiProof) {
        let mut by_leaf: BTreeMap<usize, (usize, bool)> = BTreeMap::new();
        for &i in decls {
            by_leaf.insert(self.leaf[i], (i, false));
        }
        for &i in header_only {
            let header = header_of(&self.decls[i]).is_some();
            by_leaf.insert(self.leaf[i], (i, header));
        }
        let idx: Vec<usize> = by_leaf.keys().copied().collect();
        let proof = self.tree.multiproof(&idx);
        let entries = by_leaf
            .values()
            .map(|&(i, header)| EnvEntry {
                opening: if header {
                    let h = header_of(&self.decls[i]).expect("theorem");
                    Opening::Header { header: encode_decl(arena, &h), full_hash: decl_hash(&self.bytes[i]) }
                } else {
                    Opening::Full(self.bytes[i].clone())
                },
                axioms: self.axioms[i].clone(),
            })
            .collect();
        (entries, proof)
    }

    /// Declarations the kernel may need for `user`: everything reachable through types,
    /// definition bodies and inductive blocks. With `open_theorems`, also through
    /// theorem proofs (fallback if the kernel needs to unfold a library theorem).
    /// Returns `(full, header_only)` declaration indices.
    pub fn cone(&self, arena: &Arena, user: &[Decl], open_theorems: bool) -> (Vec<usize>, Vec<usize>) {
        let mut full = alloc::collections::BTreeSet::new();
        let mut header = alloc::collections::BTreeSet::new();
        let mut work: Vec<Name> = user.iter().flat_map(|d| constants(arena, &d.exprs())).collect();
        while let Some(n) = work.pop() {
            let Some(&i) = self.by_name.get(&n) else { continue };
            if full.contains(&i) || header.contains(&i) {
                continue;
            }
            let d = &self.decls[i];
            if !open_theorems && matches!(d, Decl::Theorem { .. }) {
                header.insert(i);
                work.extend(constants(arena, &d.type_exprs()));
            } else {
                full.insert(i);
                work.extend(constants(arena, &d.exprs()));
            }
        }
        (full.into_iter().collect(), header.into_iter().collect())
    }
}
