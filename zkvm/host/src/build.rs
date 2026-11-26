//! `margin build`: prove a large development module by module.
//!
//! 1. Ask Lean which module defines each constant (a small script run with `lake env lean`).
//! 2. Export the claimed theorems with lean4export; the declarations outside the
//!    environment are the user declarations; group them by module.
//! 3. In dependency order, each module becomes a module run (`MOD1` journal): its own
//!    declarations, checked against the environment and the interfaces of the modules it
//!    uses, which it assumes as receipts. Module results are cached by content, so an
//!    unchanged module is never checked or proved twice. Independent modules run in
//!    parallel.
//! 4. The final run checks the modules that contain the claims, assumes the module
//!    receipts it uses, and commits the public journal. Proving it with the module
//!    receipts as assumptions yields one ordinary receipt.

use crate::{execute_cycles, prove_receipt, Cluster, R};
use margin_canon::{decl_hash, wire, Arena, Decl, Name, NamePart};
use margin_envbuild as eb;
use margin_methods::MARGIN_GUEST_ID;
use risc0_zkvm::sha::Digest;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

pub struct BuildOpts {
    pub project: String,
    pub env: String,
    pub root: String,
    pub claims: Vec<String>,
    pub out: String,
    pub owner: [u8; 32],
    pub cache: PathBuf,
    pub jobs: usize,
    /// Check every module natively (and fill the cache with prepared inputs), no proofs.
    pub check_only: bool,
    /// With `check_only`: also execute every run in the zkVM (no proof) and report the
    /// exact cycle counts, the input of a cost estimate.
    pub cycles: bool,
    pub cluster: Option<Cluster>,
}

/// Lean script: module names, import edges, and the module of every imported constant.
/// Names are printed as JSON arrays of components, so no escaping question arises.
const MODMAP_LEAN: &str = r#"import Lean
open Lean

def nameJson (n : Name) : String :=
  (Json.arr (n.components.toArray.map fun c => match c with
    | .str _ s => Json.str s
    | .num _ k => toJson k
    | .anonymous => Json.null)).compress

def main (args : List String) : IO Unit := do
  let root := args.head!.toName
  initSearchPath (← findSysroot)
  let env ← importModules #[{ module := root }] {} (trustLevel := 1024)
  let out ← IO.getStdout
  let names := env.header.moduleNames
  for i in [0:names.size] do
    out.putStrLn s!"M\t{i}\t{nameJson names[i]!}"
  for i in [0:env.header.moduleData.size] do
    for imp in env.header.moduleData[i]!.imports do
      if let some j := env.getModuleIdx? imp.module then
        out.putStrLn s!"I\t{i}\t{j}"
  for (n, _) in env.constants.map₁.toList do
    if let some idx := env.getModuleIdxFor? n then
      out.putStrLn s!"C\t{idx}\t{nameJson n}"
"#;

fn name_of_json(s: &str) -> R<Name> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    let parts = v.as_array().ok_or("name: not an array")?;
    Ok(Name(
        parts
            .iter()
            .map(|p| match p {
                serde_json::Value::String(s) => Ok(NamePart::Str(s.clone())),
                serde_json::Value::Number(k) => k.as_u64().map(NamePart::Num).ok_or("name: bad number"),
                _ => Err("name: bad component"),
            })
            .collect::<Result<_, _>>()?,
    ))
}

/// Constant → module name, from the Lean script.
fn module_map(project: &str, root: &str, cache: &Path) -> R<BTreeMap<Name, Name>> {
    let script = cache.join("modmap.lean");
    std::fs::write(&script, MODMAP_LEAN)?;
    let t = Instant::now();
    let output = Command::new("lake")
        .current_dir(project)
        .args(["env", "lean", "--run"])
        .arg(&script)
        .arg(root)
        .output()
        .map_err(|e| format!("running `lake env lean --run`: {e}"))?;
    if !output.status.success() {
        return Err(format!("module map script failed:\n{}", String::from_utf8_lossy(&output.stderr)).into());
    }
    let text = String::from_utf8(output.stdout)?;
    let mut modules: BTreeMap<u64, Name> = BTreeMap::new();
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let mut f = line.splitn(3, '\t');
        match (f.next(), f.next(), f.next()) {
            (Some("M"), Some(i), Some(n)) => {
                modules.insert(i.parse()?, name_of_json(n)?);
            }
            (Some("C"), Some(i), Some(n)) => {
                let m = modules.get(&i.parse()?).ok_or("constant of unknown module")?;
                map.insert(name_of_json(n)?, m.clone());
            }
            _ => {}
        }
    }
    eprintln!("module map: {} modules, {} constants ({:.1?})", modules.len(), map.len(), t.elapsed());
    Ok(map)
}

/// One module of the development.
struct Module {
    name: Name,
    /// Indices into the user declarations, in export (dependency) order.
    decls: Vec<usize>,
    /// Modules whose declarations these declarations refer to.
    deps: BTreeSet<usize>,
}

/// Shared state of the parallel build.
struct State {
    done: BTreeMap<usize, Arc<eb::ModuleIface>>,
    /// Padded zkVM cycles per module run (with `--cycles`).
    cycles: BTreeMap<usize, u64>,
    running: BTreeSet<usize>,
    failed: Option<String>,
}

fn image_id() -> [u8; 32] {
    Digest::from(MARGIN_GUEST_ID).as_bytes().try_into().expect("32 bytes")
}

fn write_axioms(axioms: &[Vec<Name>]) -> Vec<u8> {
    let mut w = wire::Writer::new();
    w.uvar(axioms.len() as u64);
    for a in axioms {
        w.names(a);
    }
    w.finish()
}

fn read_axioms(b: &[u8]) -> R<Vec<Vec<Name>>> {
    let mut r = wire::Reader::new(b);
    let n = r.count()?;
    let v = (0..n).map(|_| r.names()).collect::<Result<_, _>>()?;
    r.finish()?;
    Ok(v)
}

/// Cache key of a module run: the checker, the environment, the module's declarations
/// and the interfaces of everything it may use.
fn module_key(name: &Name, bytes: &[Vec<u8>], env_root: &[u8; 32], dep_roots: &[[u8; 32]]) -> String {
    let mut w = wire::Writer::new();
    w.raw(b"margin/module-run/v1");
    w.raw(&image_id());
    w.raw(env_root);
    w.name(name);
    w.uvar(bytes.len() as u64);
    for b in bytes {
        w.raw(&decl_hash(b));
    }
    w.uvar(dep_roots.len() as u64);
    for r in dep_roots {
        w.raw(r);
    }
    hex::encode(decl_hash(&w.finish()))
}

pub fn build(o: &BuildOpts) -> R<()> {
    let t0 = Instant::now();
    std::fs::create_dir_all(&o.cache)?;
    let mut arena = Arena::new();
    let env = eb::load_any_env(&o.env, &mut arena)?;
    let modmap = module_map(&o.project, &o.root, &o.cache)?;

    // Export the claims and everything they use; split off the environment.
    let ndjson = o.cache.join("export.ndjson");
    crate::export(&o.project, &o.root, ndjson.to_str().ok_or("path")?, &o.claims)?;
    let proj = eb::read_export(ndjson.to_str().ok_or("path")?, &mut arena)?;
    let user = eb::split_user(&env, &arena, proj)?;
    let bytes: Vec<Vec<u8>> = user.iter().map(|d| margin_canon::encode_decl(&arena, d)).collect();

    // Group by module.
    let mut index: BTreeMap<Name, usize> = BTreeMap::new();
    let mut modules: Vec<Module> = Vec::new();
    let mut module_of_decl = Vec::with_capacity(user.len());
    let mut owner_of: BTreeMap<Name, usize> = BTreeMap::new();
    for (i, d) in user.iter().enumerate() {
        let m = modmap.get(&d.key()).cloned().ok_or_else(|| format!("no module for `{}`", d.key()))?;
        let k = *index.entry(m.clone()).or_insert_with(|| {
            modules.push(Module { name: m, decls: vec![], deps: BTreeSet::new() });
            modules.len() - 1
        });
        modules[k].decls.push(i);
        module_of_decl.push(k);
        for n in d.defined_names() {
            owner_of.insert(n, k);
        }
    }
    for (i, d) in user.iter().enumerate() {
        let k = module_of_decl[i];
        for c in margin_checker::env::constants(&arena, &d.exprs()) {
            if let Some(&j) = owner_of.get(&c) {
                if j != k {
                    modules[k].deps.insert(j);
                }
            }
        }
    }
    // Modules that contain claims are checked by the final run.
    let claim_names: BTreeSet<Name> = o.claims.iter().map(|c| Name::from_dotted(c)).collect();
    let mut final_mods: BTreeSet<usize> = BTreeSet::new();
    for c in &claim_names {
        final_mods.insert(*owner_of.get(c).ok_or_else(|| format!("claim `{c}` not found in the export"))?);
    }
    for (k, m) in modules.iter().enumerate() {
        if !final_mods.contains(&k) {
            if let Some(f) = m.deps.iter().find(|d| final_mods.contains(d)) {
                return Err(format!("module `{}` depends on `{}`, which contains a claim", m.name, modules[*f].name).into());
            }
        }
    }
    // Transitive dependencies, in a topological order (Lean imports are acyclic).
    let order = topo(&modules)?;
    let mut tdeps: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); modules.len()];
    for &k in &order {
        let mut s = BTreeSet::new();
        for &d in &modules[k].deps {
            s.insert(d);
            s.extend(tdeps[d].iter().copied());
        }
        tdeps[k] = s;
    }
    let todo: Vec<usize> = order.iter().copied().filter(|k| !final_mods.contains(k)).collect();
    eprintln!(
        "{} user declarations in {} modules ({} module runs + final run) ({:.1?})",
        user.len(),
        modules.len(),
        todo.len(),
        t0.elapsed()
    );

    // Module runs, in parallel, in dependency order.
    let state = Mutex::new(State { done: BTreeMap::new(), cycles: BTreeMap::new(), running: BTreeSet::new(), failed: None });
    let cv = Condvar::new();
    let (arena, env) = (&arena, &env);
    std::thread::scope(|s| {
        for _ in 0..o.jobs.max(1) {
            s.spawn(|| loop {
                let k = {
                    let mut st = state.lock().unwrap();
                    loop {
                        if st.failed.is_some() || st.done.len() == todo.len() {
                            return;
                        }
                        let ready = todo.iter().copied().find(|k| {
                            !st.done.contains_key(k)
                                && !st.running.contains(k)
                                && modules[*k].deps.iter().all(|d| st.done.contains_key(d))
                        });
                        if let Some(k) = ready {
                            st.running.insert(k);
                            break k;
                        }
                        st = cv.wait(st).unwrap();
                    }
                };
                let deps: Vec<Arc<eb::ModuleIface>> = {
                    let st = state.lock().unwrap();
                    tdeps[k].iter().map(|d| st.done[d].clone()).collect()
                };
                let res = module_run(o, arena, env, &modules[k], &user, &bytes, &deps);
                let mut st = state.lock().unwrap();
                st.running.remove(&k);
                match res {
                    Ok((m, cycles)) => {
                        st.done.insert(k, Arc::new(m));
                        let c = cycles.map(|c| format!(", {:.1}M cycles", c as f64 / 1e6)).unwrap_or_default();
                        if let Some(c) = cycles {
                            st.cycles.insert(k, c);
                        }
                        eprintln!("[{}/{}] {} done ({} declarations{c})", st.done.len(), todo.len(), modules[k].name, modules[k].decls.len());
                    }
                    Err(e) => st.failed = Some(format!("module `{}`: {e}", modules[k].name)),
                }
                cv.notify_all();
            });
        }
    });
    let st = state.into_inner().unwrap();
    if let Some(e) = st.failed {
        return Err(e.into());
    }

    // Final run: the claim modules' declarations, over every module interface.
    let mut final_decls: Vec<usize> = final_mods.iter().flat_map(|k| modules[*k].decls.iter().copied()).collect();
    final_decls.sort_unstable();
    let final_user: Vec<Decl> = final_decls.iter().map(|&i| user[i].clone()).collect();
    let ifaces: Vec<Arc<eb::ModuleIface>> = st.done.values().cloned().collect();
    let lib = eb::Library::new(env, ifaces.iter().map(|m| m.as_ref()).collect())?;
    let job = eb::Job {
        user: &final_user,
        claims: &o.claims,
        owner: o.owner,
        module: Name::from_dotted(&o.root),
        self_id: image_id(),
    };
    let p = eb::prepare_job(&lib, arena, &job, eb::OpenMode::Minimal)?;
    let journal = eb::check(&p.input)?;
    eprintln!(
        "final run: {} declarations, {} modules assumed; native pre-check ACCEPTED ({:.1?} total)",
        p.user,
        p.input.deps.len(),
        t0.elapsed()
    );
    println!("{}", margin_verify::render_journal(&journal)?);
    let input = p.input.encode();
    std::fs::write(format!("{}.input.bin", o.out), &input)?;
    if o.check_only {
        if o.cycles {
            let fin = execute_cycles(&input)?;
            let modules_total: u64 = st.cycles.values().sum();
            let max = st.cycles.values().max().copied().unwrap_or(0);
            eprintln!(
                "cycles (padded): {} module runs {:.3}G (largest {:.1}M), final run {:.1}M, total {:.3}G",
                st.cycles.len(),
                modules_total as f64 / 1e9,
                max as f64 / 1e6,
                fin as f64 / 1e6,
                (modules_total + fin) as f64 / 1e9
            );
        }
        eprintln!("check only: wrote {}.input.bin; no proof", o.out);
        return Ok(());
    }
    let assumptions = assumed_receipts(o, &p.input, &ifaces)?;
    let (receipt, stats) = prove_receipt(&input, &assumptions, o.cluster.as_ref())?;
    if receipt.journal.bytes != journal.encode() {
        return Err("internal error: final journal differs from the native pre-check".into());
    }
    std::fs::write(&o.out, bincode::serialize(&receipt)?)?;
    eprintln!("final receipt: {stats}; wrote {} ({:.1?} total)", o.out, t0.elapsed());
    Ok(())
}

fn topo(modules: &[Module]) -> R<Vec<usize>> {
    let mut state = vec![0u8; modules.len()];
    let mut order = Vec::with_capacity(modules.len());
    fn go(k: usize, modules: &[Module], state: &mut [u8], order: &mut Vec<usize>) -> R<()> {
        match state[k] {
            2 => return Ok(()),
            1 => return Err(format!("cyclic module dependency through `{}`", modules[k].name).into()),
            _ => {}
        }
        state[k] = 1;
        for &d in &modules[k].deps {
            go(d, modules, state, order)?;
        }
        state[k] = 2;
        order.push(k);
        Ok(())
    }
    for k in 0..modules.len() {
        go(k, modules, &mut state, &mut order)?;
    }
    Ok(order)
}

/// Receipts of the modules an input assumes, by interface root, from the cache.
fn assumed_receipts(o: &BuildOpts, input: &margin_checker::ProverInput, ifaces: &[Arc<eb::ModuleIface>]) -> R<Vec<risc0_zkvm::Receipt>> {
    let by_journal: BTreeMap<&[u8], &eb::ModuleIface> = ifaces.iter().map(|m| (m.journal.as_slice(), m.as_ref())).collect();
    input
        .deps
        .iter()
        .map(|d| {
            let m = by_journal.get(d.journal.as_slice()).ok_or("assumed module not built")?;
            let path = o.cache.join(&m.key).join("receipt.bin");
            let b = std::fs::read(&path).map_err(|e| format!("{}: {e} (module `{}` not proved yet)", path.display(), m.name))?;
            Ok(bincode::deserialize(&b)?)
        })
        .collect()
}

/// Check (and unless `check_only`, prove) one module, or take it from the cache.
fn module_run(
    o: &BuildOpts,
    arena: &Arena,
    env: &margin_checker::Env,
    m: &Module,
    user: &[Decl],
    bytes: &[Vec<u8>],
    deps: &[Arc<eb::ModuleIface>],
) -> R<(eb::ModuleIface, Option<u64>)> {
    let decls: Vec<Decl> = m.decls.iter().map(|&i| user[i].clone()).collect();
    let own_bytes: Vec<Vec<u8>> = m.decls.iter().map(|&i| bytes[i].clone()).collect();
    let mut roots: Vec<[u8; 32]> = deps.iter().map(|d| d.iface.tree.root).collect();
    roots.sort();
    let key = module_key(&m.name, &own_bytes, &env.tree.root, &roots);
    let dir = o.cache.join(&key);
    let (journal_p, axioms_p, input_p, receipt_p) =
        (dir.join("journal.bin"), dir.join("axioms.bin"), dir.join("input.bin"), dir.join("receipt.bin"));
    let iface_of = |journal: Vec<u8>, axioms: Vec<Vec<Name>>| -> R<eb::ModuleIface> {
        let iface = margin_checker::Env::from_parts(arena, decls.clone(), own_bytes.clone(), axioms)?;
        let j = margin_journal::ModJournal::decode(&journal).map_err(|e| e.to_string())?;
        if j.iface_root != iface.tree.root {
            return Err("cached interface does not match its journal".into());
        }
        Ok(eb::ModuleIface { name: m.name.clone(), iface, journal, key: key.clone() })
    };

    let cycles_p = dir.join("cycles.txt");
    let cycles = |input: &[u8]| -> R<Option<u64>> {
        if !(o.check_only && o.cycles) {
            return Ok(None);
        }
        if let Ok(c) = std::fs::read_to_string(&cycles_p) {
            return Ok(Some(c.trim().parse()?));
        }
        let c = execute_cycles(input)?;
        std::fs::write(&cycles_p, c.to_string())?;
        Ok(Some(c))
    };
    // Cached: checked (journal, axioms, input) and, unless checking only, proved.
    if journal_p.exists() && axioms_p.exists() && (o.check_only || receipt_p.exists()) {
        let c = if input_p.exists() { cycles(&std::fs::read(&input_p)?)? } else { None };
        return Ok((iface_of(std::fs::read(&journal_p)?, read_axioms(&std::fs::read(&axioms_p)?)?)?, c));
    }
    std::fs::create_dir_all(&dir)?;
    let (journal, axioms, input) = if journal_p.exists() && axioms_p.exists() && input_p.exists() {
        (std::fs::read(&journal_p)?, read_axioms(&std::fs::read(&axioms_p)?)?, std::fs::read(&input_p)?)
    } else {
        let lib = eb::Library::new(env, deps.iter().map(|d| d.as_ref()).collect())?;
        let job = eb::Job { user: &decls, claims: &[], owner: [0; 32], module: m.name.clone(), self_id: image_id() };
        let p = eb::prepare_job(&lib, arena, &job, eb::OpenMode::Minimal)?;
        let (j, iface) = eb::check_module(&p.input, arena, &decls)?;
        let input = p.input.encode();
        let journal = j.encode();
        std::fs::write(&input_p, &input)?;
        std::fs::write(&axioms_p, write_axioms(&iface.axioms))?;
        std::fs::write(&journal_p, &journal)?;
        (journal, iface.axioms, input)
    };
    if !o.check_only {
        let parsed = margin_checker::ProverInput::decode(&input).map_err(|e| e.to_string())?;
        let assumptions = assumed_receipts(o, &parsed, deps)?;
        let (receipt, stats) = prove_receipt(&input, &assumptions, o.cluster.as_ref())?;
        if receipt.journal.bytes != journal {
            return Err("internal error: module journal differs from the native pre-check".into());
        }
        std::fs::write(&receipt_p, bincode::serialize(&receipt)?)?;
        eprintln!("  {}: proved ({stats})", m.name);
    }
    let c = cycles(&input)?;
    Ok((iface_of(journal, axioms)?, c))
}
