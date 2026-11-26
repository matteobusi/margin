//! Native tooling around the checker: environments (from lean4export files or a binary
//! cache), prover inputs, native pre-checks, and statement matching for verifiers.
//!
//! The `margin-envbuild` binary and the `margin` CLI are thin wrappers around this library.

use margin_canon::lean4export::read_ndjson_from;
use margin_canon::wire::{Reader, Writer};
use margin_canon::{decode_decl, decode_decl_trusted, encode_decl, Arena, Decl, Name};
use margin_checker::{public_statement, run, Env, ProverInput};
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
    pub env_full: usize,
    pub env_header: usize,
}

/// Build the prover input: user declarations, the environment cone with one multi-proof,
/// the claims and the owner key.
pub fn prepare(env: &Env, arena: &Arena, user: &[Decl], claims: &[String], owner: [u8; 32], open_theorems: bool) -> Prepared {
    let (full, header) = env.cone(arena, user, open_theorems);
    let (env_decls, env_proof) = env.open(arena, &full, &header);
    Prepared {
        input: ProverInput {
            env_root: env.tree.root,
            owner,
            claimed: claims.iter().map(|c| Name::from_dotted(c)).collect(),
            env_decls,
            env_proof,
            user_decls: user.iter().map(|d| encode_decl(arena, d)).collect(),
        },
        user: user.len(),
        env_full: full.len(),
        env_header: header.len(),
    }
}

/// Native pre-check: exactly what the guest runs, without a proof.
pub fn check(input: &ProverInput) -> R<Journal> {
    run(input, &mut NanodaKernel::default()).map_err(|e| format!("REJECTED: {e:?}"))
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
            Some((&user[i], false))
        } else {
            env.by_name.get(n).map(|&i| (&env.decls[i], true))
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
