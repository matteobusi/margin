use crate::input::ProverInput;
use crate::kernel::Kernel;
use crate::policy::{ALLOWED_AXIOMS, FORBIDDEN};
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec::Vec,
};
use crate::env::{constants, Opening};
use margin_canon::merkle::leaf_hash;
use margin_canon::{axioms_hash, decl_hash, decode_decl_trusted, encode_decl, name_hash, Arena, Decl, Name};
use margin_journal::{Journal, VERSION};

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
}

impl From<margin_canon::Error> for CheckError {
    fn from(e: margin_canon::Error) -> Self {
        CheckError::Decode(e)
    }
}

struct Entry {
    decl: Decl,
    from_env: bool,
    /// For environment declarations: their committed transitive axiom set.
    axioms: Vec<Name>,
}

/// Run the full check. The guest calls this and commits the returned journal.
pub fn run<K: Kernel>(input: &ProverInput, kernel: &mut K) -> Result<Journal, CheckError> {
    run_profiled(input, kernel, &|_| {})
}

/// [`run`] with a callback at phase boundaries (for cycle profiling in the guest).
pub fn run_profiled<K: Kernel>(
    input: &ProverInput,
    kernel: &mut K,
    mark: &dyn Fn(&str),
) -> Result<Journal, CheckError> {
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

    // 1. Environment declarations: one multi-proof against `env_root`, then add
    //    without a check. Header-only theorems become opaque constants (axioms) for
    //    the kernel; their committed axiom sets stand in for their proofs.
    let mut leaves = Vec::with_capacity(input.env_decls.len());
    let mut decoded = Vec::with_capacity(input.env_decls.len());
    for e in &input.env_decls {
        let (decl, header_hash, full_hash) = match &e.opening {
            Opening::Full(bytes) => {
                let d = decode_decl_trusted(bytes, &mut arena)?;
                let full_hash = decl_hash(bytes);
                let header_hash = crate::env::header_of(&d).map_or(full_hash, |h| decl_hash(&encode_decl(&arena, &h)));
                (d, header_hash, full_hash)
            }
            Opening::Header { header, full_hash } => {
                let Decl::Statement { name, lparams, ty } = decode_decl_trusted(header, &mut arena)? else {
                    return Err(CheckError::BadHeader);
                };
                let d = Decl::Axiom { name, lparams, ty, is_unsafe: false };
                (d, decl_hash(header), *full_hash)
            }
        };
        if matches!(decl, Decl::Statement { .. }) {
            return Err(CheckError::StatementInInput);
        }
        if e.axioms.windows(2).any(|w| w[0] >= w[1]) {
            return Err(CheckError::BadHeader);
        }
        leaves.push(leaf_hash(&name_hash(&decl.key()), &header_hash, &full_hash, &axioms_hash(&e.axioms)));
        decoded.push(decl);
    }
    mark("run: decoded env decls");
    if !input.env_proof.verify(&leaves, &input.env_root) {
        return Err(CheckError::BadInclusion(decoded.first().map(|d| d.key()).unwrap_or_default()));
    }
    for (decl, e) in decoded.into_iter().zip(&input.env_decls) {
        register(&mut by_name, &decl, entries.len())?;
        entries.push(Entry { decl, from_env: true, axioms: e.axioms.clone() });
    }
    mark("run: verified env inclusion");

    // 2. User declarations: policy checks, then the kernel.
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
        entries.push(Entry { decl, from_env: false, axioms: Vec::new() });
    }

    mark("run: user decls decoded + policy");

    // 3. The kernel checks all user declarations, in order, against the environment.
    {
        let env: Vec<Decl> = entries.iter().filter(|e| e.from_env).map(|e| e.decl.clone()).collect();
        let user: Vec<Decl> = entries.iter().filter(|e| !e.from_env).map(|e| e.decl.clone()).collect();
        kernel.check(&arena, &env, &user).map_err(|e| CheckError::Kernel { msg: e.0 })?;
    }

    let allowed: BTreeSet<Name> = ALLOWED_AXIOMS.iter().map(|s| Name::from_dotted(s)).collect();
    let forbidden: BTreeSet<Name> = FORBIDDEN.iter().map(|s| Name::from_dotted(s)).collect();

    // Public part of the journal: claimed statements and their statement cone.
    let lookup = |n: &Name| by_name.get(n).map(|&i| (&entries[i].decl, entries[i].from_env));
    let (claims, stmt_cone) = public_statement(&arena, &lookup, &input.claimed)?;

    let mut axioms_used: BTreeSet<Name> = BTreeSet::new();
    for claim in &input.claimed {
        let idx = by_name[claim];

        // 4. Axioms: walk user declarations (types and proofs); at an environment
        //    declaration, use its committed transitive axiom set.
        let mut check_axiom = |a: &Name| -> Result<(), CheckError> {
            if forbidden.contains(a) {
                return Err(CheckError::Forbidden { claim: claim.clone(), constant: a.clone() });
            }
            if !allowed.contains(a) {
                return Err(CheckError::DisallowedAxiom { claim: claim.clone(), axiom: a.clone() });
            }
            axioms_used.insert(a.clone());
            Ok(())
        };
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut work = alloc::vec![idx];
        while let Some(i) = work.pop() {
            if !seen.insert(i) {
                continue;
            }
            let e = &entries[i];
            if e.from_env {
                for a in &e.axioms {
                    check_axiom(a)?;
                }
                continue;
            }
            for c in constants(&arena, &e.decl.exprs()) {
                if forbidden.contains(&c) {
                    return Err(CheckError::Forbidden { claim: claim.clone(), constant: c });
                }
                let j = *by_name.get(&c).ok_or(CheckError::MissingDependency(c))?;
                work.push(j);
            }
        }
    }

    Ok(Journal {
        version: VERSION,
        checker: kernel.name().into(),
        env_root: input.env_root,
        owner: input.owner,
        claims,
        stmt_cone,
        axioms: axioms_used.into_iter().collect(),
    })
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
    lookup: &dyn Fn(&Name) -> Option<(&'d Decl, bool)>,
    claimed: &[Name],
) -> Result<PublicStatement, CheckError> {
    let mut claims = Vec::new();
    let mut roots: BTreeSet<Name> = BTreeSet::new();
    for claim in claimed {
        let (d, from_env) = lookup(claim).ok_or_else(|| CheckError::ClaimNotFound(claim.clone()))?;
        if from_env {
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
        let (d, from_env) = lookup(&n).ok_or_else(|| CheckError::MissingDependency(n.clone()))?;
        if from_env || !seen.insert(d.key()) {
            continue;
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
            claimed: claimed.iter().map(|s| Name::from_dotted(s)).collect(),
            env_decls,
            env_proof,
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
        let j = run(&inp, &mut Yes).unwrap();
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
        let j = run(&inp, &mut Yes).unwrap();
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
            claimed: vec![Name::from_dotted("t")],
            env_decls,
            env_proof,
            user_decls: vec![encode_decl(&f.arena, &t)],
        };
        (inp, f)
    }

    #[test]
    fn statement_only_library_theorem_is_accepted() {
        let (inp, _) = header_input(false);
        assert!(inp.env_decls.iter().any(|e| matches!(e.opening, crate::Opening::Header { .. })));
        let j = run(&inp, &mut Yes).unwrap();
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
}