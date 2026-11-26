//! Native tooling around the checker: environments (from lean4export files or a binary
//! cache), prover inputs, native pre-checks, and statement matching for verifiers.
//!
//! The `margin-envbuild` binary and the `margin` CLI are thin wrappers around this library.

use margin_canon::lean4export::read_ndjson_from;
use margin_canon::wire::{Reader, Writer};
use margin_canon::{decode_decl, decode_decl_trusted, encode_decl, Arena, Decl, Name};
use margin_checker::env::{constants, header_of};
use margin_checker::{public_statement, run, DepOpening, Env, Origin, ProverInput};
use margin_journal::Journal;
use margin_kernel_nanoda::NanodaKernel;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, BufWriter, Read, Write};
use std::time::Instant;

pub type R<T> = Result<T, String>;

/// Magic of the environment cache file.
const ENV_MAGIC: &[u8; 4] = b"ENV1";

/// Read a lean4export file (streaming; works for Mathlib-sized exports).
pub fn read_export(path: &str, arena: &mut Arena) -> R<Vec<Decl>> {
    let t = Instant::now();
    let f = std::fs::File::open(path).map_err(|e| format!("{path}: {e}"))?;
    let decls = read_ndjson_from(BufReader::with_capacity(1 << 20, f), arena, &mut |n| {
        eprintln!("  {path}: {}M lines", n / 1_000_000)
    })
    .map_err(|e| format!("{path}: {e}"))?;
    eprintln!("read {path}: {} declarations, {} expr nodes ({:.1?})", decls.len(), arena.len(), t.elapsed());
    Ok(decls)
}

/// Build an environment from a lean4export file.
pub fn env_from_export(path: &str, arena: &mut Arena) -> R<Env> {
    let decls = read_export(path, arena)?;
    let t = Instant::now();
    let env = Env::build(arena, decls)?;
    eprintln!("built environment tree: {} declarations ({:.1?})", env.decls.len(), t.elapsed());
    Ok(env)
}

/// Save an environment as a binary cache (CE bytes + axiom sets + root).
pub fn save_env(env: &Env, path: &str) -> R<()> {
    let f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let mut out = BufWriter::new(f);
    let mut w = Writer::new();
    w.raw(ENV_MAGIC);
    w.raw(&env.tree.root);
    w.uvar(env.decls.len() as u64);
    out.write_all(&w.finish()).map_err(|e| e.to_string())?;
    for (b, ax) in env.bytes.iter().zip(&env.axioms) {
        let mut w = Writer::new();
        w.bytes(b);
        w.names(ax);
        out.write_all(&w.finish()).map_err(|e| e.to_string())?;
    }
    out.flush().map_err(|e| e.to_string())
}

/// Load an environment cache. Every leaf is recomputed, and the root must match the one
/// recorded in the file.
pub fn load_env(path: &str, arena: &mut Arena) -> R<Env> {
    let t = Instant::now();
    let mut data = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut data))
        .map_err(|e| format!("{path}: {e}"))?;
    let mut r = Reader::new(&data);
    let e = |x: margin_canon::Error| format!("{path}: {x}");
    if r.raw(4).map_err(e)? != ENV_MAGIC {
        return Err(format!("{path}: not an environment cache"));
    }
    let root = r.array32().map_err(e)?;
    let n = r.count().map_err(e)?;
    let (mut decls, mut bytes, mut axioms) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for _ in 0..n {
        let b = r.bytes().map_err(e)?;
        decls.push(decode_decl_trusted(b, arena).map_err(e)?);
        bytes.push(b.to_vec());
        axioms.push(r.names().map_err(e)?);
    }
    r.finish().map_err(e)?;
    drop(data);
    let env = Env::from_parts(arena, decls, bytes, axioms)?;
    if env.tree.root != root {
        return Err(format!("{path}: recorded root does not match its contents"));
    }
    eprintln!("loaded environment {path}: {} declarations ({:.1?})", env.decls.len(), t.elapsed());
    Ok(env)
}

/// Load an environment from a lean4export file (`.ndjson`) or a cache (anything else).
pub fn load_any_env(path: &str, arena: &mut Arena) -> R<Env> {
    if path.ends_with(".ndjson") || path.ends_with(".json") {
        env_from_export(path, arena)
    } else {
        load_env(path, arena)
    }
}

/// User declarations of a project export: everything not in the environment. Exported
/// declarations that are in the environment must be byte-identical to it.
pub fn split_user(env: &Env, arena: &Arena, proj: Vec<Decl>) -> R<Vec<Decl>> {
    let mut user = Vec::new();
    for d in proj {
        match env.by_name.get(&d.key()) {
            Some(&i) => {
                if encode_decl(arena, &d) != env.bytes[i] {
                    return Err(format!(
                        "`{}` differs from the environment (different Lean or Mathlib version?)",
                        d.key()
                    ));
                }
            }
            None => user.push(d),
        }
    }
    Ok(user)
}

pub struct Prepared {
    pub input: ProverInput,
    pub user: usize,
    /// Opened declarations, in full and as statements, over the whole library.
    pub env_full: usize,
    pub env_header: usize,
}

/// Which library declarations to open in full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// Only what the kernel uses: a recorded native run decides (the default).
    Minimal,
    /// Everything reachable, theorems as statements unless `open_theorems`.
    Reachable { open_theorems: bool },
}

/// A proved module: its interface (an [`Env`] of its own declarations with their axiom
/// sets; same leaf format as the environment) and the journal of its receipt.
pub struct ModuleIface {
    pub name: Name,
    pub iface: Env,
    /// `MOD1` journal bytes of the module receipt.
    pub journal: Vec<u8>,
    /// Cache key of the module run (where its receipt is stored).
    pub key: String,
}

/// Declarations a run may use: the trusted environment (source 0) and the interfaces of
/// proved modules (sources 1..).
pub struct Library<'a> {
    pub env: &'a Env,
    pub mods: Vec<&'a ModuleIface>,
    by_name: BTreeMap<Name, (usize, usize)>,
}

impl<'a> Library<'a> {
    pub fn new(env: &'a Env, mods: Vec<&'a ModuleIface>) -> R<Self> {
        let mut by_name = BTreeMap::new();
        for (k, m) in mods.iter().enumerate() {
            for (n, &i) in &m.iface.by_name {
                if env.by_name.contains_key(n) || by_name.insert(n.clone(), (k + 1, i)).is_some() {
                    return Err(format!("`{n}` is defined twice (module `{}`)", m.name));
                }
            }
        }
        Ok(Self { env, mods, by_name })
    }

    fn source(&self, src: usize) -> &Env {
        if src == 0 {
            self.env
        } else {
            &self.mods[src - 1].iface
        }
    }

    /// `(source, declaration index)` of a constant.
    pub fn lookup(&self, n: &Name) -> Option<(usize, usize)> {
        self.env.by_name.get(n).map(|&i| (0, i)).or_else(|| self.by_name.get(n).copied())
    }

    fn decl(&self, (src, i): (usize, usize)) -> &Decl {
        &self.source(src).decls[i]
    }
}

type Loc = (usize, usize);

/// Library declarations a run needs for `user`, as `(full, header_only)` sets: every
/// constant reachable from the user declarations; through full openings also from
/// definition bodies and inductive blocks. `used` (minimal mode) lists what the kernel
/// consulted; `force_full` must be opened in full whatever the mode.
fn cone(
    lib: &Library,
    arena: &Arena,
    user: &[Decl],
    mode: OpenMode,
    used: &BTreeSet<Loc>,
    force_full: &BTreeSet<Loc>,
) -> (BTreeSet<Loc>, BTreeSet<Loc>) {
    let mut full = BTreeSet::new();
    let mut header = BTreeSet::new();
    let mut work: Vec<Name> = user.iter().flat_map(|d| constants(arena, &d.exprs())).collect();
    while let Some(n) = work.pop() {
        let Some(loc) = lib.lookup(&n) else { continue };
        if full.contains(&loc) || header.contains(&loc) {
            continue;
        }
        let d = lib.decl(loc);
        let as_header = header_of(d).is_some()
            && !force_full.contains(&loc)
            && match mode {
                OpenMode::Reachable { open_theorems } => !open_theorems && matches!(d, Decl::Theorem { .. }),
                OpenMode::Minimal => !used.contains(&loc),
            };
        if as_header {
            header.insert(loc);
            work.extend(constants(arena, &d.type_exprs()));
        } else {
            full.insert(loc);
            work.extend(constants(arena, &d.exprs()));
        }
    }
    (full, header)
}

/// Module declarations the public statement of `claims` shows (definitions in full,
/// theorems as statements): they must be opened in full.
fn statement_modules(lib: &Library, arena: &Arena, user: &[Decl], claims: &[String]) -> BTreeSet<Loc> {
    let user_by_name: BTreeMap<Name, &Decl> =
        user.iter().flat_map(|d| d.defined_names().into_iter().map(move |n| (n, d))).collect();
    let mut out = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut work: Vec<Name> = claims
        .iter()
        .filter_map(|c| user_by_name.get(&Name::from_dotted(c)))
        .flat_map(|d| constants(arena, &d.type_exprs()))
        .collect();
    while let Some(n) = work.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        let d = if let Some(d) = user_by_name.get(&n) {
            *d
        } else if let Some(loc @ (src, _)) = lib.lookup(&n) {
            if src == 0 {
                continue;
            }
            out.insert(loc);
            lib.decl(loc)
        } else {
            continue;
        };
        let follow = if matches!(d, Decl::Theorem { .. }) { d.type_exprs() } else { d.exprs() };
        work.extend(constants(arena, &follow));
    }
    out
}

/// What to prove: the user declarations of one run, and how to label its output.
pub struct Job<'a> {
    pub user: &'a [Decl],
    /// Empty: module mode (the run commits a module journal).
    pub claims: &'a [String],
    pub owner: [u8; 32],
    pub module: Name,
    /// Image ID of the checker that will prove this run (and that proved the modules).
    pub self_id: [u8; 32],
}

fn build_input(lib: &Library, arena: &Arena, job: &Job, full: &BTreeSet<Loc>, header: &BTreeSet<Loc>) -> Prepared {
    let pick = |set: &BTreeSet<Loc>, src: usize| -> Vec<usize> { set.iter().filter(|l| l.0 == src).map(|l| l.1).collect() };
    let (env_decls, env_proof) = lib.env.open(arena, &pick(full, 0), &pick(header, 0));
    let mut deps = Vec::new();
    for (k, m) in lib.mods.iter().enumerate() {
        let (f, h) = (pick(full, k + 1), pick(header, k + 1));
        if f.is_empty() && h.is_empty() {
            continue;
        }
        let (entries, proof) = m.iface.open(arena, &f, &h);
        deps.push(DepOpening { journal: m.journal.clone(), entries, proof });
    }
    Prepared {
        input: ProverInput {
            env_root: lib.env.tree.root,
            owner: job.owner,
            self_id: job.self_id,
            module: job.module.clone(),
            claimed: job.claims.iter().map(|c| Name::from_dotted(c)).collect(),
            env_decls,
            env_proof,
            deps,
            user_decls: job.user.iter().map(|d| encode_decl(arena, d)).collect(),
        },
        user: job.user.len(),
        env_full: full.len(),
        env_header: header.len(),
    }
}

/// Build the prover input of `job` over `lib`. With [`OpenMode::Minimal`]: prepare
/// everything reachable, run the checker natively while recording which library
/// declarations the kernel consults, prepare again opening only those in full, and check
/// that input natively too (falling back to the reachable input if it is rejected).
/// Soundness does not depend on this choice: the guest checks whatever it is given.
pub fn prepare_job(lib: &Library, arena: &Arena, job: &Job, mode: OpenMode) -> R<Prepared> {
    let force = if job.claims.is_empty() { BTreeSet::new() } else { statement_modules(lib, arena, job.user, job.claims) };
    let none = BTreeSet::new();
    let reach_mode = match mode {
        OpenMode::Minimal => OpenMode::Reachable { open_theorems: false },
        m => m,
    };
    let (full, header) = cone(lib, arena, job.user, reach_mode, &none, &force);
    let reachable = build_input(lib, arena, job, &full, &header);
    if mode != OpenMode::Minimal {
        return Ok(reachable);
    }
    let t = Instant::now();
    let mut kernel = NanodaKernel { record_value_uses: true, ..Default::default() };
    run(&reachable.input, &mut kernel).map_err(|e| format!("REJECTED: {e:?}"))?;
    // The kernel's environment is the opened entries in input order: environment, then
    // each module's entries.
    let opened: Vec<&margin_checker::EnvEntry> =
        reachable.input.env_decls.iter().chain(reachable.input.deps.iter().flat_map(|d| d.entries.iter())).collect();
    let used: BTreeSet<Loc> = kernel
        .value_uses
        .iter()
        .filter_map(|&i| {
            let bytes = match &opened.get(i)?.opening {
                margin_checker::Opening::Full { bytes, .. } => bytes,
                margin_checker::Opening::Header { header, .. } => header,
            };
            let mut a = Arena::new();
            lib.lookup(&decode_decl_trusted(bytes, &mut a).ok()?.key())
        })
        .collect();
    eprintln!(
        "native check (recording): accepted, kernel consulted {} of {} opened declarations ({:.1?})",
        used.len(),
        reachable.env_full,
        t.elapsed()
    );
    let (full, header) = cone(lib, arena, job.user, OpenMode::Minimal, &used, &force);
    let minimal = build_input(lib, arena, job, &full, &header);
    let t = Instant::now();
    match run(&minimal.input, &mut NanodaKernel::default()) {
        Ok(_) => {
            eprintln!("native check (minimal openings): accepted ({:.1?})", t.elapsed());
            Ok(minimal)
        }
        Err(e) => {
            eprintln!("warning: minimal openings rejected ({e:?}); using all reachable declarations");
            Ok(reachable)
        }
    }
}

/// Single-run prover input over the environment alone (no modules): the claims are
/// proved in one receipt.
pub fn prepare(env: &Env, arena: &Arena, user: &[Decl], claims: &[String], owner: [u8; 32], mode: OpenMode) -> R<Prepared> {
    let lib = Library::new(env, vec![])?;
    let job = Job { user, claims, owner, module: Name::from_dotted("Main"), self_id: [0; 32] };
    prepare_job(&lib, arena, &job, mode)
}

/// Native pre-check of a claim run: exactly what the guest runs, without a proof (and
/// without verifying assumed module receipts).
pub fn check(input: &ProverInput) -> R<Journal> {
    run(input, &mut NanodaKernel::default())
        .and_then(|r| r.journal())
        .map_err(|e| format!("REJECTED: {e:?}"))
}

/// Native run of a module job: returns the module journal and the module's interface,
/// whose root must equal the journal's (it is what importers open against).
pub fn check_module(input: &ProverInput, arena: &Arena, user: &[Decl]) -> R<(margin_journal::ModJournal, Env)> {
    let r = run(input, &mut NanodaKernel::default()).map_err(|e| format!("REJECTED: {e:?}"))?;
    let margin_checker::Output::Module(j) = r.output else {
        return Err("not a module run".into());
    };
    let iface = Env::from_parts(arena, user.to_vec(), input.user_decls.clone(), r.user_axioms)?;
    if iface.tree.root != j.iface_root {
        return Err("internal error: interface root differs from the module journal".into());
    }
    Ok((j, iface))
}

/// Compare a journal (from a verified receipt) with the verifier's own statement export.
/// Returns a human-readable MATCH report, or the list of mismatches.
pub fn match_journal(journal: &Journal, env: &Env, arena: &mut Arena, stmt: Vec<Decl>, claims: &[String]) -> R<String> {
    if env.tree.root != journal.env_root {
        return Err(format!(
            "MISMATCH: environment differs (yours {}, receipt {})",
            hex::encode(env.tree.root),
            hex::encode(journal.env_root)
        ));
    }
    let user = split_user(env, arena, stmt)?;
    let mut user_by_name = BTreeMap::new();
    for (i, d) in user.iter().enumerate() {
        for n in d.defined_names() {
            user_by_name.insert(n, i);
        }
    }
    let lookup = |n: &Name| {
        if let Some(&i) = user_by_name.get(n) {
            Some((&user[i], Origin::User))
        } else {
            env.by_name.get(n).map(|&i| (&env.decls[i], Origin::Env))
        }
    };
    let claimed: Vec<Name> = claims.iter().map(|c| Name::from_dotted(c)).collect();
    let (exp_claims, exp_cone) = public_statement(arena, &lookup, &claimed).map_err(|e| format!("{e:?}"))?;

    let mut problems = Vec::new();
    let as_set = |v: &[Vec<u8>]| v.iter().cloned().collect::<BTreeSet<_>>();
    let got_claims = as_set(&journal.claims);
    for (i, c) in exp_claims.iter().enumerate() {
        if !got_claims.contains(c) {
            problems.push(format!("claim `{}`: the receipt does not prove this exact statement", claims[i]));
        }
    }
    // Definitions by name: same name with different bytes, or present on one side only.
    let by_key = |set: &BTreeSet<Vec<u8>>| {
        set.iter()
            .map(|b| {
                let mut a = Arena::new();
                let k = decode_decl(b, &mut a).map(|d| d.key().to_string()).unwrap_or_else(|_| "<undecodable>".into());
                (k, b.clone())
            })
            .collect::<BTreeMap<String, Vec<u8>>>()
    };
    let (mine, theirs) = (by_key(&as_set(&exp_cone)), by_key(&as_set(&journal.stmt_cone)));
    for (k, b) in &mine {
        match theirs.get(k) {
            Some(t) if t == b => {}
            Some(_) => problems.push(format!("definition `{k}`: your definition differs from the receipt's")),
            None => problems.push(format!("definition `{k}`: used by your statement but not in the receipt")),
        }
    }
    for k in theirs.keys().filter(|k| !mine.contains_key(*k)) {
        problems.push(format!("definition `{k}`: in the receipt but not used by your statement"));
    }
    if exp_claims.len() != journal.claims.len() {
        problems.push(format!("the receipt claims {} theorems, you asked about {}", journal.claims.len(), exp_claims.len()));
    }
    if problems.is_empty() {
        Ok(format!(
            "MATCH: the receipt proves exactly your statements, with exactly your definitions,\nover environment {}.\nAxioms used: [{}]",
            hex::encode(journal.env_root),
            journal.axioms.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")
        ))
    } else {
        Err(format!("MISMATCH:\n  {}", problems.join("\n  ")))
    }
}
