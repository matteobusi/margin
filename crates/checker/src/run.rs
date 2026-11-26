use crate::input::ProverInput;
use crate::kernel::Kernel;
use crate::policy::{ALLOWED_AXIOMS, FORBIDDEN};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use crate::env::{constants, EnvEntry, Opening};
use margin_canon::merkle::{leaf_hash, MultiProof};
use margin_canon::{axioms_hash, decl_hash, decode_decl_trusted, encode_decl, name_hash, Arena, Decl, Hash32, Name};
use margin_journal::{Journal, ModJournal, VERSION};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    Decode(margin_canon::Error),
    BadInclusion(Name),
    BadHeader,
    DuplicateName(Name),
    UnknownConstant { decl: Name, constant: Name },
    UserAxiom(Name),
    UnsafeDecl(Name),
    StatementInInput,
    Kernel { msg: String },
    ClaimNotFound(Name),
    ClaimNotTheorem(Name),
    ClaimFromEnvironment(Name),
    Forbidden { claim: Name, constant: Name },
    DisallowedAxiom { claim: Name, axiom: Name },
    MissingDependency(Name),
    /// An assumed module receipt is malformed, for another image or environment, or
    /// did not verify.
    BadDependency(String),
    /// A definition the public statement needs was opened only as a statement.
    HiddenStatementDependency(Name),
}

impl From<margin_canon::Error> for CheckError {
    fn from(e: margin_canon::Error) -> Self {
        CheckError::Decode(e)
    }
}

/// Where a declaration of a run comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The trusted environment (`env_root`).
    Env,
    /// An assumed module receipt, opened in full.
    Dep,
    /// An assumed module receipt, opened as a statement only.
    DepHeader,
    /// This run's own declarations.
    User,
}

struct Entry {
    decl: Decl,
    origin: Origin,
    /// Transitive axiom set: committed (environment, modules) or computed (user).
    axioms: Vec<Name>,
}

/// What a run commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Claim mode: the public journal.
    Final(Journal),
    /// Module mode: the module journal.
    Module(ModJournal),
}

impl Output {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Output::Final(j) => j.encode(),
            Output::Module(m) => m.encode(),
        }
    }
}

/// A run's output, plus the computed axiom set of every user declaration (in input
/// order). The axiom sets are not committed; native tools use them to rebuild a module's
/// interface tree.
#[derive(Clone, Debug)]
pub struct RunResult {
    pub output: Output,
    pub user_axioms: Vec<Vec<Name>>,
}

impl RunResult {
    /// The public journal (claim mode).
    pub fn journal(self) -> Result<Journal, CheckError> {
        match self.output {
            Output::Final(j) => Ok(j),
            Output::Module(_) => Err(CheckError::BadDependency("module run has no public journal".into())),
        }
    }
}

/// Run the full check without verifying assumed module receipts: for native pre-checks.
/// The guest uses [`run_profiled`] with a verifier for them.
pub fn run<K: Kernel>(input: &ProverInput, kernel: &mut K) -> Result<RunResult, CheckError> {
    run_profiled(input, kernel, &|_| {}, &mut |_, _| true)
}

/// Decoded openings (with a flag for statement-only ones) and their leaf hashes.
type Opened = (Vec<(Decl, bool)>, Vec<Hash32>);

/// Decode openings and compute their leaves. Header openings become axioms of the same
/// type for the kernel; their committed axiom sets stand in for their values.
fn open_entries(entries: &[EnvEntry], arena: &mut Arena) -> Result<Opened, CheckError> {
    let mut leaves = Vec::with_capacity(entries.len());
    let mut decoded = Vec::with_capacity(entries.len());
    for e in entries {
        let (decl, header_hash, full_hash, header_only) = match &e.opening {
            Opening::Full(bytes) => {
                let d = decode_decl_trusted(bytes, arena)?;
                let full_hash = decl_hash(bytes);
                let header_hash = crate::env::header_of(&d).map_or(full_hash, |h| decl_hash(&encode_decl(arena, &h)));
                (d, header_hash, full_hash, false)
            }
            Opening::Header { header, full_hash } => {
                let Decl::Statement { name, lparams, ty } = decode_decl_trusted(header, arena)? else {
                    return Err(CheckError::BadHeader);
                };
                let d = Decl::Axiom { name, lparams, ty, is_unsafe: false };
                (d, decl_hash(header), *full_hash, true)
            }
        };
        if matches!(decl, Decl::Statement { .. }) {
            return Err(CheckError::StatementInInput);
        }
        if e.axioms.windows(2).any(|w| w[0] >= w[1]) {
            return Err(CheckError::BadHeader);
        }
        leaves.push(leaf_hash(&name_hash(&decl.key()), &header_hash, &full_hash, &axioms_hash(&e.axioms)));
        decoded.push((decl, header_only));
    }
    Ok((decoded, leaves))
}

fn verify_openings(proof: &MultiProof, leaves: &[Hash32], root: &Hash32, first: Option<&Decl>) -> Result<(), CheckError> {
    if leaves.is_empty() || proof.verify(leaves, root) {
        Ok(())
    } else {
        Err(CheckError::BadInclusion(first.map(|d| d.key()).unwrap_or_default()))
    }
}

/// [`run`] with a callback at phase boundaries (for cycle profiling in the guest) and a
/// verifier for assumed module receipts: `verify_dep(image_id, journal)` must succeed
/// only if a receipt of `image_id` with that journal exists (in the guest:
/// `env::verify`, which turns it into an assumption of the final receipt).
pub fn run_profiled<K: Kernel>(
    input: &ProverInput,
    kernel: &mut K,
    mark: &dyn Fn(&str),
    verify_dep: &mut dyn FnMut(&Hash32, &[u8]) -> bool,
) -> Result<RunResult, CheckError> {
    let mut arena = Arena::new();
    let mut entries: Vec<Entry> = Vec::new();
    let mut by_name: BTreeMap<Name, usize> = BTreeMap::new();

    let register = |by_name: &mut BTreeMap<Name, usize>, d: &Decl, idx: usize| {
        for n in d.defined_names() {
            if by_name.insert(n.clone(), idx).is_some() {
                return Err(CheckError::DuplicateName(n));
            }
        }
        Ok(())
    };

    // 1. Environment declarations: one multi-proof against `env_root`, then added
    //    without a check.
    let (decoded, leaves) = open_entries(&input.env_decls, &mut arena)?;
    mark("run: decoded env decls");
    verify_openings(&input.env_proof, &leaves, &input.env_root, decoded.first().map(|(d, _)| d))?;
    for ((decl, _), e) in decoded.into_iter().zip(&input.env_decls) {
        register(&mut by_name, &decl, entries.len())?;
        entries.push(Entry { decl, origin: Origin::Env, axioms: e.axioms.clone() });
    }
    mark("run: verified env inclusion");

    // 2. Assumed module receipts: same image, same environment; openings against each
    //    module's interface root.
    let mut dep_roots = Vec::with_capacity(input.deps.len());
    for dep in &input.deps {
        let j = ModJournal::decode(&dep.journal).map_err(|e| CheckError::BadDependency(alloc::format!("{e}")))?;
        if j.self_id != input.self_id || j.env_root != input.env_root {
            return Err(CheckError::BadDependency(alloc::format!("`{}`: other image or environment", j.module)));
        }
        if dep.proof.size != j.iface_size {
            return Err(CheckError::BadDependency(alloc::format!("`{}`: interface size", j.module)));
        }
        if !verify_dep(&input.self_id, &dep.journal) {
            return Err(CheckError::BadDependency(alloc::format!("`{}`: receipt does not verify", j.module)));
        }
        let (decoded, leaves) = open_entries(&dep.entries, &mut arena)?;
        verify_openings(&dep.proof, &leaves, &j.iface_root, decoded.first().map(|(d, _)| d))?;
        for ((decl, header_only), e) in decoded.into_iter().zip(&dep.entries) {
            register(&mut by_name, &decl, entries.len())?;
            let origin = if header_only { Origin::DepHeader } else { Origin::Dep };
            entries.push(Entry { decl, origin, axioms: e.axioms.clone() });
        }
        dep_roots.push(j.iface_root);
    }
    mark("run: verified module dependencies");

    // 3. User declarations: policy checks, then the kernel.
    let first_user = entries.len();
    for bytes in &input.user_decls {
        let decl = decode_decl_trusted(bytes, &mut arena)?;
        let key = decl.key();
        match &decl {
            Decl::Statement { .. } => return Err(CheckError::StatementInInput),
            Decl::Axiom { .. } => return Err(CheckError::UserAxiom(key)),
            // The quotient constants must come from the environment.
            Decl::Quot { .. } => return Err(CheckError::UserAxiom(key)),
            d if d.is_unsafe() => return Err(CheckError::UnsafeDecl(key)),
            _ => {}
        }
        // Every referenced constant must already exist (or be defined by this block).
        let own: BTreeSet<Name> = decl.defined_names().into_iter().collect();
        for c in constants(&arena, &decl.exprs()) {
            if !by_name.contains_key(&c) && !own.contains(&c) {
                return Err(CheckError::UnknownConstant { decl: key, constant: c });
            }
        }
        register(&mut by_name, &decl, entries.len())?;
        entries.push(Entry { decl, origin: Origin::User, axioms: Vec::new() });
    }
    mark("run: user decls decoded + policy");

    // 4. The kernel checks all user declarations, in order, against the environment and
    //    the assumed modules.
    {
        let env: Vec<Decl> = entries[..first_user].iter().map(|e| e.decl.clone()).collect();
        let user: Vec<Decl> = entries[first_user..].iter().map(|e| e.decl.clone()).collect();
        kernel.check(&arena, &env, &user).map_err(|e| CheckError::Kernel { msg: e.0 })?;
    }

    // 5. Transitive axiom sets of the user declarations, by the environment's rule: the
    //    union over referenced constants (a user declaration is never an axiom). User
    //    declarations only refer to earlier ones, or to their own block.
    for i in first_user..entries.len() {
        let mut set: Vec<Name> = Vec::new();
        for c in constants(&arena, &entries[i].decl.exprs()) {
            let j = *by_name.get(&c).ok_or(CheckError::MissingDependency(c))?;
            if j != i {
                set.extend(entries[j].axioms.iter().cloned());
            }
        }
        set.sort();
        set.dedup();
        entries[i].axioms = set;
    }
    let user_axioms: Vec<Vec<Name>> = entries[first_user..].iter().map(|e| e.axioms.clone()).collect();

    // 6a. Module mode: commit the interface of the user declarations.
    if input.claimed.is_empty() {
        let decls: Vec<Decl> = entries[first_user..].iter().map(|e| e.decl.clone()).collect();
        let iface = crate::env::Env::from_parts(&arena, decls, input.user_decls.clone(), user_axioms.clone())
            .map_err(CheckError::BadDependency)?;
        mark("checker: module interface built");
        let output = Output::Module(ModJournal {
            self_id: input.self_id,
            env_root: input.env_root,
            module: input.module.clone(),
            iface_root: iface.tree.root,
            iface_size: iface.tree.leaves.len() as u64,
            deps: dep_roots,
        });
        return Ok(RunResult { output, user_axioms });
    }

    // 6b. Claim mode: public statement and axiom policy.
    let allowed: BTreeSet<Name> = ALLOWED_AXIOMS.iter().map(|s| Name::from_dotted(s)).collect();
    let forbidden: BTreeSet<Name> = FORBIDDEN.iter().map(|s| Name::from_dotted(s)).collect();
    let lookup = |n: &Name| by_name.get(n).map(|&i| (&entries[i].decl, entries[i].origin));
    let (claims, stmt_cone) = public_statement(&arena, &lookup, &input.claimed)?;

    let mut axioms_used: BTreeSet<Name> = BTreeSet::new();
    for claim in &input.claimed {
        for a in &entries[by_name[claim]].axioms {
            if forbidden.contains(a) {
                return Err(CheckError::Forbidden { claim: claim.clone(), constant: a.clone() });
            }
            if !allowed.contains(a) {
                return Err(CheckError::DisallowedAxiom { claim: claim.clone(), axiom: a.clone() });
            }
            axioms_used.insert(a.clone());
        }
    }

    let output = Output::Final(Journal {
        version: VERSION,
        checker: kernel.name().into(),
        env_root: input.env_root,
        owner: input.owner,
        claims,
        stmt_cone,
        axioms: axioms_used.into_iter().collect(),
        assumed_id: if input.deps.is_empty() { [0; 32] } else { input.self_id },
    });
    Ok(RunResult { output, user_axioms })
}

/// `(claims, stmt_cone)`: CE bytes as committed in the journal.
pub type PublicStatement = (Vec<Vec<u8>>, Vec<Vec<u8>>);

/// The public part of a journal for `claimed`: the CE `Statement` of each claim (in
/// order) and the statement cone (user declarations the statements depend on, sorted by
/// name; theorems in the cone appear as statements only).
///
/// `lookup` resolves a constant name to its declaration and whether it comes from the
/// trusted environment. The guest and `match` both use this function, so a verifier
/// recomputes exactly what the guest committed.
pub fn public_statement<'d>(
    arena: &Arena,
    lookup: &dyn Fn(&Name) -> Option<(&'d Decl, Origin)>,
    claimed: &[Name],
) -> Result<PublicStatement, CheckError> {
    let mut claims = Vec::new();
    let mut roots: BTreeSet<Name> = BTreeSet::new();
    for claim in claimed {
        let (d, origin) = lookup(claim).ok_or_else(|| CheckError::ClaimNotFound(claim.clone()))?;
        if origin != Origin::User {
            return Err(CheckError::ClaimFromEnvironment(claim.clone()));
        }
        let Decl::Theorem { name, lparams, ty, .. } = d else {
            return Err(CheckError::ClaimNotTheorem(claim.clone()));
        };
        let stmt = Decl::Statement { name: name.clone(), lparams: lparams.clone(), ty: *ty };
        claims.push(encode_decl(arena, &stmt));
        roots.extend(constants(arena, &[*ty]));
    }

    let mut cone: BTreeMap<Name, Vec<u8>> = BTreeMap::new();
    let mut seen: BTreeSet<Name> = BTreeSet::new();
    let mut work: Vec<Name> = roots.into_iter().collect();
    while let Some(n) = work.pop() {
        let (d, origin) = lookup(&n).ok_or_else(|| CheckError::MissingDependency(n.clone()))?;
        if origin == Origin::Env || !seen.insert(d.key()) {
            continue;
        }
        // A module declaration in the statement cone must be opened in full: otherwise
        // a definition could appear as a bare statement and hide what it means.
        if origin == Origin::DepHeader {
            return Err(CheckError::HiddenStatementDependency(d.key()));
        }
        let (bytes, follow) = match d {
            // Theorems: statement only. Never reveal the proof.
            Decl::Theorem { name, lparams, ty, .. } => {
                let s = Decl::Statement { name: name.clone(), lparams: lparams.clone(), ty: *ty };
                (encode_decl(arena, &s), d.type_exprs())
            }
            _ => (encode_decl(arena, d), d.exprs()),
        };
        cone.insert(d.key(), bytes);
        work.extend(constants(arena, &follow));
    }
    Ok((claims, cone.into_values().collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::KernelError;
    use alloc::vec;
    use margin_canon::{CtorVal, IndType, InductiveBlock, Level};

    /// Accepts everything. Tests cover the pipeline, not type checking.
    struct Yes;
    impl Kernel for Yes {
        fn name(&self) -> &str {
            "test"
        }
        fn check(&mut self, _: &Arena, _: &[Decl], _: &[Decl]) -> Result<(), KernelError> {
            Ok(())
        }
    }

    struct Fixture {
        arena: Arena,
        env: Vec<Decl>,
    }

    fn fixture() -> Fixture {
        let mut a = Arena::new();
        let prop = a.sort(Level::Zero);
        let ty = a.sort(Level::Succ(alloc::boxed::Box::new(Level::Zero)));
        let nat = a.cnst("Nat", vec![]);
        let nat_block = Decl::Inductive(InductiveBlock {
            types: vec![IndType {
                name: Name::from_dotted("Nat"),
                lparams: vec![],
                ty,
                all: vec![Name::from_dotted("Nat")],
                ctors: vec![Name::from_dotted("Nat.zero")],
                is_rec: false,
                is_reflexive: false,
                is_unsafe: false,
                num_params: 0,
                num_indices: 0,
                num_nested: 0,
            }],
            ctors: vec![CtorVal {
                name: Name::from_dotted("Nat.zero"),
                lparams: vec![],
                ty: nat,
                is_unsafe: false,
                induct: Name::from_dotted("Nat"),
                cidx: 0,
                num_params: 0,
                num_fields: 0,
            }],
            recs: vec![],
        });
        let sorry = Decl::Axiom { name: Name::from_dotted("sorryAx"), lparams: vec![], ty: prop, is_unsafe: false };
        let propext = Decl::Axiom { name: Name::from_dotted("propext"), lparams: vec![], ty: prop, is_unsafe: false };
        let true_c = a.cnst("True", vec![]);
        let true_block = Decl::Inductive(InductiveBlock {
            types: vec![IndType {
                name: Name::from_dotted("True"),
                lparams: vec![],
                ty: prop,
                all: vec![Name::from_dotted("True")],
                ctors: vec![Name::from_dotted("True.intro")],
                is_rec: false,
                is_reflexive: false,
                is_unsafe: false,
                num_params: 0,
                num_indices: 0,
                num_nested: 0,
            }],
            ctors: vec![CtorVal {
                name: Name::from_dotted("True.intro"),
                lparams: vec![],
                ty: true_c,
                is_unsafe: false,
                induct: Name::from_dotted("True"),
                cidx: 0,
                num_params: 0,
                num_fields: 0,
            }],
            recs: vec![],
        });
        Fixture { arena: a, env: vec![nat_block, sorry, propext, true_block] }
    }

    fn input(f: &mut Fixture, user: Vec<Decl>, claimed: &[&str]) -> ProverInput {
        let env = crate::Env::build(&f.arena, f.env.clone()).unwrap();
        let all: Vec<usize> = (0..f.env.len()).collect();
        let (env_decls, env_proof) = env.open(&f.arena, &all, &[]);
        ProverInput {
            env_root: env.tree.root,
            owner: [1; 32],
            self_id: [0; 32],
            module: Name::from_dotted("Test"),
            claimed: claimed.iter().map(|s| Name::from_dotted(s)).collect(),
            env_decls,
            env_proof,
            deps: vec![],
            user_decls: user.iter().map(|d| encode_decl(&f.arena, d)).collect(),
        }
    }

    fn thm(f: &mut Fixture, name: &str, ty: &str, val: &str) -> Decl {
        let ty = f.arena.cnst(ty, vec![]);
        let val = f.arena.cnst(val, vec![]);
        Decl::Theorem { name: Name::from_dotted(name), lparams: vec![], ty, val }
    }

    #[test]
    fn accepts_clean_theorem_and_hides_proof() {
        let mut f = fixture();
        let helper = thm(&mut f, "helper", "True", "True.intro");
        let main = thm(&mut f, "main", "True", "helper");
        let inp = input(&mut f, vec![helper, main], &["main"]);
        let j = run(&inp, &mut Yes).unwrap().journal().unwrap();
        assert_eq!(j.claims.len(), 1);
        assert!(j.stmt_cone.is_empty(), "statement uses only env constants");
        // The journal never contains the name of the private lemma.
        let bytes = j.encode();
        assert!(!bytes.windows(6).any(|w| w == b"helper"));
        assert!(j.axioms.is_empty());
    }

    #[test]
    fn rejects_sorry() {
        let mut f = fixture();
        let bad = thm(&mut f, "bad", "True", "sorryAx");
        let inp = input(&mut f, vec![bad], &["bad"]);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::Forbidden { .. })));
    }

    #[test]
    fn rejects_user_axiom_and_unknown_constant() {
        let mut f = fixture();
        let prop = f.arena.sort(Level::Zero);
        let ax = Decl::Axiom { name: Name::from_dotted("cheat"), lparams: vec![], ty: prop, is_unsafe: false };
        let inp = input(&mut f, vec![ax], &[]);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::UserAxiom(_))));

        let t = thm(&mut f, "t", "True", "does_not_exist");
        let inp = input(&mut f, vec![t], &["t"]);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::UnknownConstant { .. })));
    }

    #[test]
    fn rejects_tampered_environment() {
        let mut f = fixture();
        let t = thm(&mut f, "t", "True", "propext");
        let mut inp = input(&mut f, vec![t], &["t"]);
        inp.env_root[0] ^= 1;
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::BadInclusion(_))));
    }

    #[test]
    fn records_allowed_axioms_and_reveals_statement_definitions() {
        let mut f = fixture();
        // def MyProp := True ; theorem t : MyProp := propext
        let true_c = f.arena.cnst("True", vec![]);
        let prop = f.arena.sort(Level::Zero);
        let my = Decl::Definition {
            name: Name::from_dotted("MyProp"),
            lparams: vec![],
            ty: prop,
            val: true_c,
            hints: margin_canon::Hints::Abbrev,
            safety: margin_canon::Safety::Safe,
        };
        let t = thm(&mut f, "t", "MyProp", "propext");
        let inp = input(&mut f, vec![my, t], &["t"]);
        let j = run(&inp, &mut Yes).unwrap().journal().unwrap();
        assert_eq!(j.axioms, vec![Name::from_dotted("propext")]);
        assert_eq!(j.stmt_cone.len(), 1, "MyProp must be public");
    }

    #[test]
    fn input_roundtrip() {
        let mut f = fixture();
        let t = thm(&mut f, "t", "True", "propext");
        let inp = input(&mut f, vec![t], &["t"]);
        assert_eq!(ProverInput::decode(&inp.encode()).unwrap(), inp);
    }

    /// Environment with `env_ok : True := True.intro` and `env_bad : True := sorryAx`,
    /// opened statement-only; user theorem `t : True := <env theorem>`.
    fn header_input(use_bad: bool) -> (ProverInput, Fixture) {
        let mut f = fixture();
        let true_c = f.arena.cnst("True", vec![]);
        let intro = f.arena.cnst("True.intro", vec![]);
        let sorry = f.arena.cnst("sorryAx", vec![]);
        f.env.push(Decl::Theorem { name: Name::from_dotted("env_ok"), lparams: vec![], ty: true_c, val: intro });
        f.env.push(Decl::Theorem { name: Name::from_dotted("env_bad"), lparams: vec![], ty: true_c, val: sorry });
        let env = crate::Env::build(&f.arena, f.env.clone()).unwrap();
        let theorems: Vec<usize> = (f.env.len() - 2..f.env.len()).collect();
        let others: Vec<usize> = (0..f.env.len() - 2).collect();
        let (env_decls, env_proof) = env.open(&f.arena, &others, &theorems);
        let used = if use_bad { "env_bad" } else { "env_ok" };
        let t = thm(&mut f, "t", "True", used);
        let inp = ProverInput {
            env_root: env.tree.root,
            owner: [1; 32],
            self_id: [0; 32],
            module: Name::from_dotted("Test"),
            claimed: vec![Name::from_dotted("t")],
            env_decls,
            env_proof,
            deps: vec![],
            user_decls: vec![encode_decl(&f.arena, &t)],
        };
        (inp, f)
    }

    #[test]
    fn statement_only_library_theorem_is_accepted() {
        let (inp, _) = header_input(false);
        assert!(inp.env_decls.iter().any(|e| matches!(e.opening, crate::Opening::Header { .. })));
        let j = run(&inp, &mut Yes).unwrap().journal().unwrap();
        assert!(j.axioms.is_empty());
    }

    #[test]
    fn hidden_library_proof_still_contributes_its_axioms() {
        // `env_bad` is opened without its proof, but its committed axiom set contains
        // `sorryAx`, so a claim that uses it is rejected.
        let (inp, _) = header_input(true);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::Forbidden { .. })));
    }

    #[test]
    fn tampered_header_or_axiom_set_is_rejected() {
        let (inp, _) = header_input(true);
        // Drop `sorryAx` from the committed axiom set of `env_bad`.
        let mut a = inp.clone();
        for e in &mut a.env_decls {
            e.axioms.retain(|n| *n != Name::from_dotted("sorryAx"));
        }
        assert!(matches!(run(&a, &mut Yes), Err(CheckError::BadInclusion(_))));
        // Claim a different type for a statement-only theorem.
        let mut b = inp.clone();
        for e in &mut b.env_decls {
            if let crate::Opening::Header { header, .. } = &mut e.opening {
                let mut ar = Arena::new();
                if let Decl::Statement { name, lparams, .. } = margin_canon::decode_decl(header, &mut ar).unwrap() {
                    let nat2 = ar.cnst("Nat", vec![]);
                    *header = encode_decl(&ar, &Decl::Statement { name, lparams, ty: nat2 });
                }
            }
        }
        assert!(matches!(run(&b, &mut Yes), Err(CheckError::BadInclusion(_))));
    }

    // ---- Composition: module runs assumed by a claim run. ----

    const ID: [u8; 32] = [7; 32];

    /// A module run over the fixture environment: its journal bytes and interface.
    fn module_of(f: &Fixture, decls: Vec<Decl>, self_id: [u8; 32]) -> (Vec<u8>, crate::Env) {
        let env = crate::Env::build(&f.arena, f.env.clone()).unwrap();
        let all: Vec<usize> = (0..f.env.len()).collect();
        let (env_decls, env_proof) = env.open(&f.arena, &all, &[]);
        let user_decls: Vec<Vec<u8>> = decls.iter().map(|d| encode_decl(&f.arena, d)).collect();
        let inp = ProverInput {
            env_root: env.tree.root,
            owner: [0; 32],
            self_id,
            module: Name::from_dotted("M"),
            claimed: vec![],
            env_decls,
            env_proof,
            deps: vec![],
            user_decls: user_decls.clone(),
        };
        let r = run(&inp, &mut Yes).unwrap();
        let Output::Module(j) = r.output else { panic!("module run must commit a module journal") };
        assert_eq!(j.self_id, self_id);
        let iface = crate::Env::from_parts(&f.arena, decls, user_decls, r.user_axioms).unwrap();
        assert_eq!(iface.tree.root, j.iface_root, "native interface = committed interface");
        (j.encode(), iface)
    }

    /// A claim run that assumes one module, opening `full` / `header` of its interface.
    fn claim_over(f: &Fixture, journal: Vec<u8>, iface: &crate::Env, full: &[usize], header: &[usize], user: Vec<Decl>, self_id: [u8; 32]) -> ProverInput {
        let env = crate::Env::build(&f.arena, f.env.clone()).unwrap();
        let all: Vec<usize> = (0..f.env.len()).collect();
        let (env_decls, env_proof) = env.open(&f.arena, &all, &[]);
        let (entries, proof) = iface.open(&f.arena, full, header);
        ProverInput {
            env_root: env.tree.root,
            owner: [1; 32],
            self_id,
            module: Name::from_dotted("Main"),
            claimed: vec![Name::from_dotted("main")],
            env_decls,
            env_proof,
            deps: vec![crate::DepOpening { journal, entries, proof }],
            user_decls: user.iter().map(|d| encode_decl(&f.arena, d)).collect(),
        }
    }

    /// Module: `def d : Prop := True` and `theorem lem : True := True.intro`.
    /// Claim: `theorem main : d := lem`.
    fn module_fixture() -> (Fixture, Vec<u8>, crate::Env, Decl) {
        let mut f = fixture();
        let prop = f.arena.sort(Level::Zero);
        let true_c = f.arena.cnst("True", vec![]);
        let d = Decl::Definition {
            name: Name::from_dotted("d"),
            lparams: vec![],
            ty: prop,
            val: true_c,
            hints: margin_canon::Hints::Abbrev,
            safety: margin_canon::Safety::Safe,
        };
        let lem = thm(&mut f, "lem", "True", "True.intro");
        let (journal, iface) = module_of(&f, vec![d, lem], ID);
        let main = thm(&mut f, "main", "d", "lem");
        (f, journal, iface, main)
    }

    fn idx(iface: &crate::Env, n: &str) -> usize {
        iface.by_name[&Name::from_dotted(n)]
    }

    #[test]
    fn claim_over_module_commits_assumed_id_and_reveals_definitions() {
        let (f, journal, iface, main) = module_fixture();
        let (d, lem) = (idx(&iface, "d"), idx(&iface, "lem"));
        let inp = claim_over(&f, journal, &iface, &[d], &[lem], vec![main], ID);
        let mut asked = Vec::new();
        let r = run_profiled(&inp, &mut Yes, &|_| {}, &mut |id, j| {
            asked.push((*id, j.to_vec()));
            true
        })
        .unwrap();
        let j = r.journal().unwrap();
        assert_eq!(j.assumed_id, ID);
        assert_eq!(asked.len(), 1, "the module receipt is verified (assumed) once");
        assert_eq!(asked[0].0, ID);
        // The statement needs `d`: revealed in full. `lem` (a proof) is not revealed.
        assert_eq!(j.stmt_cone.len(), 1);
        let mut a = Arena::new();
        assert!(matches!(margin_canon::decode_decl(&j.stmt_cone[0], &mut a).unwrap(), Decl::Definition { .. }));
        // A single run without modules commits a zero assumed_id.
        let single = run(&input(&mut fixture(), vec![], &[]), &mut Yes);
        assert!(single.is_ok());
    }

    #[test]
    fn module_of_another_image_or_environment_is_rejected() {
        let (f, journal, iface, main) = module_fixture();
        let all: Vec<usize> = (0..iface.decls.len()).collect();
        let inp = claim_over(&f, journal.clone(), &iface, &all, &[], vec![main.clone()], [9; 32]);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::BadDependency(_))));
        // Same image, but the module journal names another environment root.
        let mut j = margin_journal::ModJournal::decode(&journal).unwrap();
        j.env_root = [5; 32];
        let inp = claim_over(&f, j.encode(), &iface, &all, &[], vec![main], ID);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::BadDependency(_))));
    }

    #[test]
    fn unverified_module_receipt_is_rejected() {
        let (f, journal, iface, main) = module_fixture();
        let all: Vec<usize> = (0..iface.decls.len()).collect();
        let inp = claim_over(&f, journal, &iface, &all, &[], vec![main], ID);
        let r = run_profiled(&inp, &mut Yes, &|_| {}, &mut |_, _| false);
        assert!(matches!(r, Err(CheckError::BadDependency(_))));
    }

    #[test]
    fn tampered_module_opening_is_rejected() {
        let (f, journal, iface, main) = module_fixture();
        let all: Vec<usize> = (0..iface.decls.len()).collect();
        let mut inp = claim_over(&f, journal, &iface, &all, &[], vec![main], ID);
        // Drop an axiom from a committed set: the leaf changes, the proof fails.
        inp.deps[0].entries[0].axioms.push(Name::from_dotted("zzz"));
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::BadInclusion(_))));
    }

    #[test]
    fn hidden_definition_in_statement_is_rejected() {
        let (f, journal, iface, main) = module_fixture();
        let (d, lem) = (idx(&iface, "d"), idx(&iface, "lem"));
        let inp = claim_over(&f, journal, &iface, &[], &[d, lem], vec![main], ID);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::HiddenStatementDependency(_))));
    }

    #[test]
    fn sorry_inside_a_module_still_reaches_the_claim() {
        let mut f = fixture();
        let bad = thm(&mut f, "bad", "True", "sorryAx");
        let (journal, iface) = module_of(&f, vec![bad], ID);
        let main = thm(&mut f, "main", "True", "bad");
        let b = idx(&iface, "bad");
        let inp = claim_over(&f, journal, &iface, &[], &[b], vec![main], ID);
        assert!(matches!(run(&inp, &mut Yes), Err(CheckError::Forbidden { .. })));
    }
}
