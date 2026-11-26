use crate::fxhash::FxHasher;
use alloc::{boxed::Box, string::String, vec, vec::Vec};
use core::fmt;
use core::hash::{Hash, Hasher};
use hashbrown::HashTable;

/// One component of a hierarchical Lean name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NamePart {
    Str(String),
    Num(u64),
}

/// A hierarchical Lean name, outermost component first (`Nat.add` = `[Nat, add]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(pub Vec<NamePart>);

impl Name {
    pub fn anon() -> Self {
        Name(Vec::new())
    }

    /// Parse a dotted name. Every component becomes a string component.
    /// No escaping (`«»`) support: use the constructor for unusual names.
    pub fn from_dotted(s: &str) -> Self {
        if s.is_empty() {
            return Self::anon();
        }
        Name(s.split('.').map(|p| NamePart::Str(p.into())).collect())
    }

    pub fn child(&self, part: &str) -> Self {
        let mut v = self.0.clone();
        v.push(NamePart::Str(part.into()));
        Name(v)
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str("[anonymous]");
        }
        for (i, p) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(".")?;
            }
            match p {
                NamePart::Str(s) => f.write_str(s)?,
                NamePart::Num(n) => write!(f, "{n}")?,
            }
        }
        Ok(())
    }
}

/// Universe levels.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Zero,
    Succ(Box<Level>),
    Max(Box<Level>, Box<Level>),
    IMax(Box<Level>, Box<Level>),
    Param(Name),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BinderInfo {
    Default = 0,
    Implicit = 1,
    StrictImplicit = 2,
    InstImplicit = 3,
}

impl BinderInfo {
    pub fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Default,
            1 => Self::Implicit,
            2 => Self::StrictImplicit,
            3 => Self::InstImplicit,
            _ => return None,
        })
    }
}

/// Index of an expression node in an [`Arena`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExprId(pub u32);

/// Index of an interned [`Name`] in an [`Arena`]. `NameId(0)` is the anonymous name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NameId(pub u32);

/// Index of an interned universe level in an [`Arena`]. `LevelId(0)` is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelId(pub u32);

/// Index of an interned list of universe levels. `LevelsId(0)` is the empty list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelsId(pub u32);

/// One interned universe level node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LevelNode {
    Zero,
    Succ(LevelId),
    Max(LevelId, LevelId),
    IMax(LevelId, LevelId),
    Param(NameId),
}

/// One expression node. Children are [`ExprId`]s into the same arena; names and levels
/// are interned. Binder names and binder info are not stored: they do not affect type
/// checking, Lean's own term equality ignores them, and exporters do not preserve them
/// reliably (a shared subterm can carry the binder names of another declaration).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ExprNode {
    BVar(u64),
    Sort(LevelId),
    Const(NameId, LevelsId),
    App(ExprId, ExprId),
    Lam(ExprId, ExprId),
    Pi(ExprId, ExprId),
    /// type, value, body, non-dependent
    Let(ExprId, ExprId, ExprId, bool),
    /// Big-endian, minimal (zero is empty).
    NatLit(Box<[u8]>),
    StrLit(Box<str>),
    Proj(NameId, u64, ExprId),
}

impl ExprNode {
    pub fn children(&self) -> Vec<ExprId> {
        match self {
            ExprNode::App(f, a) => vec![*f, *a],
            ExprNode::Lam(t, b) | ExprNode::Pi(t, b) => vec![*t, *b],
            ExprNode::Let(t, v, b, _) => vec![*t, *v, *b],
            ExprNode::Proj(_, _, e) => vec![*e],
            _ => Vec::new(),
        }
    }
}

fn fx<T: Hash + ?Sized>(x: &T) -> u64 {
    let mut h = FxHasher::default();
    x.hash(&mut h);
    h.finish()
}

/// A hash-consing table: each distinct value is stored once and gets a dense `u32` id.
#[derive(Clone, Debug)]
struct Interner<T> {
    items: Vec<T>,
    index: HashTable<u32>,
}

impl<T> Default for Interner<T> {
    fn default() -> Self {
        Self { items: Vec::new(), index: HashTable::new() }
    }
}

impl<T: Hash + Eq> Interner<T> {
    fn get_or_insert(&mut self, value: T) -> u32 {
        let h = fx(&value);
        let items = &self.items;
        if let Some(&i) = self.index.find(h, |&i| items[i as usize] == value) {
            return i;
        }
        let i = self.items.len() as u32;
        self.items.push(value);
        let items = &self.items;
        self.index.insert_unique(h, i, |&j| fx(&items[j as usize]));
        i
    }
}

/// Hash-consed term store. Structurally equal nodes get the same [`ExprId`]; names,
/// universe levels and level lists are interned too.
#[derive(Clone, Debug)]
pub struct Arena {
    nodes: Interner<ExprNode>,
    names: Interner<Name>,
    levels: Interner<LevelNode>,
    lists: Interner<Box<[LevelId]>>,
    /// Encoded name bytes → interned name. Lets the decoder skip building a `Name` for
    /// names it has seen before (most constant references).
    raw_names: HashTable<(Box<[u8]>, NameId)>,
}

impl Default for Arena {
    fn default() -> Self {
        let mut a = Arena {
            nodes: Interner::default(),
            names: Interner::default(),
            levels: Interner::default(),
            lists: Interner::default(),
            raw_names: HashTable::new(),
        };
        a.names.get_or_insert(Name::anon());
        a.levels.get_or_insert(LevelNode::Zero);
        a.lists.get_or_insert(Box::new([]));
        a
    }
}

impl Arena {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node (or find the existing equal node).
    ///
    /// Panics if a child or interned id is not in the arena, or if a nat literal is not
    /// minimal.
    pub fn mk(&mut self, node: ExprNode) -> ExprId {
        for c in node.children() {
            assert!((c.0 as usize) < self.nodes.items.len(), "child not in arena");
        }
        match &node {
            ExprNode::NatLit(b) => assert!(b.first() != Some(&0), "nat literal not minimal"),
            ExprNode::Sort(l) => assert!((l.0 as usize) < self.levels.items.len()),
            ExprNode::Const(n, ls) => {
                assert!((n.0 as usize) < self.names.items.len());
                assert!((ls.0 as usize) < self.lists.items.len());
            }
            ExprNode::Proj(n, _, _) => assert!((n.0 as usize) < self.names.items.len()),
            _ => {}
        }
        ExprId(self.nodes.get_or_insert(node))
    }

    pub fn get(&self, id: ExprId) -> &ExprNode {
        &self.nodes.items[id.0 as usize]
    }

    /// Number of expression nodes.
    pub fn len(&self) -> usize {
        self.nodes.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.items.is_empty()
    }

    pub fn intern_name(&mut self, n: &Name) -> NameId {
        // Avoid cloning when the name is already present.
        let h = fx(n);
        let items = &self.names.items;
        if let Some(&i) = self.names.index.find(h, |&i| items[i as usize] == *n) {
            return NameId(i);
        }
        NameId(self.names.get_or_insert(n.clone()))
    }

    /// Intern a name given its wire encoding (see [`crate::wire::Reader::name_raw`]).
    pub fn intern_name_raw(&mut self, raw: &[u8]) -> Result<NameId, crate::wire::Error> {
        let h = fx(raw);
        if let Some((_, id)) = self.raw_names.find(h, |(b, _)| **b == *raw) {
            return Ok(*id);
        }
        let mut r = crate::wire::Reader::new(raw);
        let name = r.name()?;
        r.finish()?;
        let id = self.intern_name(&name);
        self.raw_names.insert_unique(h, (raw.into(), id), |(b, _)| fx(&**b));
        Ok(id)
    }

    pub fn name(&self, id: NameId) -> &Name {
        &self.names.items[id.0 as usize]
    }

    pub fn mk_level(&mut self, l: LevelNode) -> LevelId {
        match l {
            LevelNode::Succ(a) => assert!((a.0 as usize) < self.levels.items.len()),
            LevelNode::Max(a, b) | LevelNode::IMax(a, b) => {
                assert!((a.0 as usize) < self.levels.items.len() && (b.0 as usize) < self.levels.items.len())
            }
            LevelNode::Param(n) => assert!((n.0 as usize) < self.names.items.len()),
            LevelNode::Zero => {}
        }
        LevelId(self.levels.get_or_insert(l))
    }

    pub fn level_node(&self, id: LevelId) -> LevelNode {
        self.levels.items[id.0 as usize]
    }

    /// Intern an owned level tree.
    pub fn intern_level(&mut self, l: &Level) -> LevelId {
        let node = match l {
            Level::Zero => LevelNode::Zero,
            Level::Succ(a) => LevelNode::Succ(self.intern_level(a)),
            Level::Max(a, b) => LevelNode::Max(self.intern_level(a), self.intern_level(b)),
            Level::IMax(a, b) => LevelNode::IMax(self.intern_level(a), self.intern_level(b)),
            Level::Param(n) => LevelNode::Param(self.intern_name(n)),
        };
        self.mk_level(node)
    }

    /// Rebuild an owned level tree.
    pub fn level(&self, id: LevelId) -> Level {
        match self.level_node(id) {
            LevelNode::Zero => Level::Zero,
            LevelNode::Succ(a) => Level::Succ(Box::new(self.level(a))),
            LevelNode::Max(a, b) => Level::Max(Box::new(self.level(a)), Box::new(self.level(b))),
            LevelNode::IMax(a, b) => Level::IMax(Box::new(self.level(a)), Box::new(self.level(b))),
            LevelNode::Param(n) => Level::Param(self.name(n).clone()),
        }
    }

    pub fn intern_levels(&mut self, ls: &[LevelId]) -> LevelsId {
        for l in ls {
            assert!((l.0 as usize) < self.levels.items.len());
        }
        LevelsId(self.lists.get_or_insert(ls.into()))
    }

    pub fn levels(&self, id: LevelsId) -> &[LevelId] {
        &self.lists.items[id.0 as usize]
    }

    /// Number of interned levels (for sizing per-level tables).
    pub fn level_count(&self) -> usize {
        self.levels.items.len()
    }

    /// Number of interned names (for sizing per-name tables).
    pub fn name_count(&self) -> usize {
        self.names.items.len()
    }

    // Convenience constructors.
    pub fn sort(&mut self, l: Level) -> ExprId {
        let l = self.intern_level(&l);
        self.mk(ExprNode::Sort(l))
    }
    pub fn cnst(&mut self, n: &str, ls: Vec<Level>) -> ExprId {
        self.const_named(&Name::from_dotted(n), &ls)
    }
    pub fn const_named(&mut self, n: &Name, ls: &[Level]) -> ExprId {
        let n = self.intern_name(n);
        let ls: Vec<LevelId> = ls.iter().map(|l| self.intern_level(l)).collect();
        let ls = self.intern_levels(&ls);
        self.mk(ExprNode::Const(n, ls))
    }
    pub fn app(&mut self, f: ExprId, a: ExprId) -> ExprId {
        self.mk(ExprNode::App(f, a))
    }
    pub fn bvar(&mut self, i: u64) -> ExprId {
        self.mk(ExprNode::BVar(i))
    }
    /// `Π (_ : ty), body`. The binder name is accepted for readability and dropped.
    pub fn pi(&mut self, _name: &str, ty: ExprId, body: ExprId) -> ExprId {
        self.mk(ExprNode::Pi(ty, body))
    }
    /// `λ (_ : ty), body`. The binder name is accepted for readability and dropped.
    pub fn lam(&mut self, _name: &str, ty: ExprId, body: ExprId) -> ExprId {
        self.mk(ExprNode::Lam(ty, body))
    }
    pub fn nat_lit(&mut self, v: u64) -> ExprId {
        let bytes: Vec<u8> = v.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        self.mk(ExprNode::NatLit(bytes.into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Safety {
    Safe = 0,
    Unsafe = 1,
    Partial = 2,
}

/// Reducibility hints. They guide unfolding during definitional equality checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hints {
    Opaque,
    Abbrev,
    Regular(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotKind {
    Type = 0,
    Ctor = 1,
    Lift = 2,
    Ind = 3,
}

/// One inductive type of a (mutual) inductive block. Mirrors lean4export `InductiveVal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndType {
    pub name: Name,
    pub lparams: Vec<Name>,
    pub ty: ExprId,
    pub all: Vec<Name>,
    pub ctors: Vec<Name>,
    pub is_rec: bool,
    pub is_reflexive: bool,
    pub is_unsafe: bool,
    pub num_params: u64,
    pub num_indices: u64,
    pub num_nested: u64,
}

/// Mirrors lean4export `ConstructorVal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorVal {
    pub name: Name,
    pub lparams: Vec<Name>,
    pub ty: ExprId,
    pub is_unsafe: bool,
    pub induct: Name,
    pub cidx: u64,
    pub num_params: u64,
    pub num_fields: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecRule {
    pub ctor: Name,
    pub nfields: u64,
    pub rhs: ExprId,
}

/// Mirrors lean4export `RecursorVal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecVal {
    pub name: Name,
    pub lparams: Vec<Name>,
    pub ty: ExprId,
    pub is_unsafe: bool,
    pub all: Vec<Name>,
    pub num_params: u64,
    pub num_indices: u64,
    pub num_motives: u64,
    pub num_minors: u64,
    pub rules: Vec<RecRule>,
    pub k: bool,
}

/// An inductive block exactly as Lean exports it: the types, their constructors and
/// their recursors. The kernel re-derives constructors and recursors and compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InductiveBlock {
    pub types: Vec<IndType>,
    pub ctors: Vec<CtorVal>,
    pub recs: Vec<RecVal>,
}

/// A kernel declaration. Expressions live in an external [`Arena`].
///
/// The fields are the ones a kernel reads (lean4export format 3.1). Fields that do not
/// change type checking (for example `all` of definitions) are not included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    Axiom { name: Name, lparams: Vec<Name>, ty: ExprId, is_unsafe: bool },
    Definition { name: Name, lparams: Vec<Name>, ty: ExprId, val: ExprId, hints: Hints, safety: Safety },
    Theorem { name: Name, lparams: Vec<Name>, ty: ExprId, val: ExprId },
    Opaque { name: Name, lparams: Vec<Name>, ty: ExprId, val: ExprId, is_unsafe: bool },
    /// One of the four built-in quotient constants.
    Quot { name: Name, lparams: Vec<Name>, ty: ExprId, kind: QuotKind },
    Inductive(InductiveBlock),
    /// A theorem statement without proof. Journal only.
    Statement { name: Name, lparams: Vec<Name>, ty: ExprId },
}

impl Decl {
    /// The name that identifies this declaration in an environment tree.
    pub fn key(&self) -> Name {
        match self {
            Decl::Axiom { name, .. }
            | Decl::Definition { name, .. }
            | Decl::Theorem { name, .. }
            | Decl::Opaque { name, .. }
            | Decl::Quot { name, .. }
            | Decl::Statement { name, .. } => name.clone(),
            Decl::Inductive(b) => b.types.first().map(|t| t.name.clone()).unwrap_or_default(),
        }
    }

    /// All constant names that this declaration adds to the environment.
    pub fn defined_names(&self) -> Vec<Name> {
        match self {
            Decl::Inductive(b) => b
                .types
                .iter()
                .map(|t| t.name.clone())
                .chain(b.ctors.iter().map(|c| c.name.clone()))
                .chain(b.recs.iter().map(|r| r.name.clone()))
                .collect(),
            _ => vec![self.key()],
        }
    }

    /// Root expressions in field order. CE uses this order for the expression table.
    pub fn exprs(&self) -> Vec<ExprId> {
        match self {
            Decl::Axiom { ty, .. } | Decl::Statement { ty, .. } | Decl::Quot { ty, .. } => vec![*ty],
            Decl::Definition { ty, val, .. }
            | Decl::Theorem { ty, val, .. }
            | Decl::Opaque { ty, val, .. } => vec![*ty, *val],
            Decl::Inductive(b) => {
                let mut v: Vec<ExprId> = b.types.iter().map(|t| t.ty).collect();
                v.extend(b.ctors.iter().map(|c| c.ty));
                for r in &b.recs {
                    v.push(r.ty);
                    v.extend(r.rules.iter().map(|x| x.rhs));
                }
                v
            }
        }
    }

    /// Type-only root expressions (no proof or definition bodies).
    pub fn type_exprs(&self) -> Vec<ExprId> {
        match self {
            Decl::Axiom { ty, .. }
            | Decl::Statement { ty, .. }
            | Decl::Definition { ty, .. }
            | Decl::Theorem { ty, .. }
            | Decl::Opaque { ty, .. }
            | Decl::Quot { ty, .. } => vec![*ty],
            Decl::Inductive(_) => self.exprs(),
        }
    }

    /// True if any part of the declaration is marked unsafe.
    pub fn is_unsafe(&self) -> bool {
        match self {
            Decl::Axiom { is_unsafe, .. } | Decl::Opaque { is_unsafe, .. } => *is_unsafe,
            Decl::Definition { safety, .. } => *safety == Safety::Unsafe,
            Decl::Inductive(b) => {
                b.types.iter().any(|t| t.is_unsafe)
                    || b.ctors.iter().any(|c| c.is_unsafe)
                    || b.recs.iter().any(|r| r.is_unsafe)
            }
            _ => false,
        }
    }
}
