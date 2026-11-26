//! `margin` — prove that Lean accepts your proof, without revealing the proof.
//!
//! ```text
//! margin keygen  <owner.key>                                   new prover key (pseudonymous)
//! margin export  [--project DIR] <Module> <out.ndjson> <decl>... export with lean4export
//! margin env     cache <env.ndjson> <out.cache> | root <env>   environment cache / root
//! margin prepare [--key K] [--open-theorems] <env> <project.ndjson> <input.bin> <claim>...
//! margin check   <input.bin>                                   native pre-check, no proof
//! margin execute <input.bin>                                   run the guest without proving: cycle count
//! margin prove   <input.bin> <receipt.bin>                     zkVM proof
//! margin run     [--project DIR] [--key K] [--open-theorems] <env> <Module> <receipt.bin> <claim>...
//!                                                           export + prepare + check + prove
//! margin verify  [--checkers F] [--envs F] [--claim C] <receipt.bin>
//! margin match   [--checkers F] [--envs F] <receipt.bin> <env> <statement.ndjson> <claim>...
//! margin claim   [--note TEXT] <receipt.bin> <owner.key>       signed authorship claim (JSON)
//! margin image-id | demo <receipt.bin>
//! ```
//!
//! `<env>` is an environment cache (from `margin env cache`) or a lean4export file.
//! `lean4export` is found via `$MARGIN_LEAN4EXPORT` or `PATH`.
//! The default trust lists (accepted checkers and environments) are built in; override
//! them with `--checkers` / `--envs`.

use margin_canon::*;
use margin_checker::ProverInput;
use margin_envbuild as eb;
use margin_methods::{MARGIN_GUEST_ELF, MARGIN_GUEST_ID};
use risc0_zkvm::{default_prover, sha::Digest, ExecutorEnv, ProverOpts};
use std::process::{Command, ExitCode};

type R<T> = Result<T, Box<dyn std::error::Error>>;

const DEFAULT_CHECKERS: &str = include_str!("../../../web/public/checkers.json");
const DEFAULT_ENVS: &str = include_str!("../../../web/public/envs.json");

fn image_id_hex() -> String {
    hex::encode(Digest::from(MARGIN_GUEST_ID).as_bytes())
}

fn prove(input: &[u8], out: &str) -> R<()> {
    let env = ExecutorEnv::builder()
        .write_slice(&[u32::try_from(input.len())?])
        .write_slice(input)
        .build()?;
    let t = std::time::Instant::now();
    // Succinct: one constant-size receipt. Composite receipts leak the segment count.
    let info = default_prover().prove_with_opts(env, MARGIN_GUEST_ELF, &ProverOpts::succinct())?;
    info.receipt.verify(MARGIN_GUEST_ID)?;
    std::fs::write(out, bincode::serialize(&info.receipt)?)?;
    eprintln!("cycles:   {}", info.stats.total_cycles);
    eprintln!("time:     {:.1?}", t.elapsed());
    eprintln!("image id: {}", image_id_hex());
    eprintln!("wrote {out}");
    Ok(())
}

/// Run the guest without proving (fast): reports cycles, so users can estimate cost.
fn execute(input: &[u8]) -> R<()> {
    let env = ExecutorEnv::builder()
        .write_slice(&[u32::try_from(input.len())?])
        .write_slice(input)
        .build()?;
    let t = std::time::Instant::now();
    let info = risc0_zkvm::default_executor().execute(env, MARGIN_GUEST_ELF)?;
    let cycles: u64 = info.segments.iter().map(|s| 1u64 << s.po2).sum();
    eprintln!("executed in {:.1?}: {} segments, ~{} cycles (padded)", t.elapsed(), info.segments.len(), cycles);
    let journal = margin_journal::Journal::decode(&info.journal.bytes).map_err(|e| e.to_string())?;
    println!("{}", margin_verify::render_journal(&journal)?);
    Ok(())
}

/// Parsed arguments: positional values plus `--flag value` / `--switch` options.
struct Args {
    pos: Vec<String>,
    opts: std::collections::BTreeMap<String, String>,
    switches: std::collections::BTreeSet<String>,
}

fn parse(args: Vec<String>, with_value: &[&str], switches: &[&str]) -> R<Args> {
    let mut a = Args { pos: vec![], opts: Default::default(), switches: Default::default() };
    let mut it = args.into_iter();
    while let Some(x) = it.next() {
        if with_value.contains(&x.as_str()) {
            let v = it.next().ok_or_else(|| format!("{x} needs a value"))?;
            a.opts.insert(x, v);
        } else if switches.contains(&x.as_str()) {
            a.switches.insert(x);
        } else if x.starts_with("--") {
            return Err(format!("unknown option {x}").into());
        } else {
            a.pos.push(x);
        }
    }
    Ok(a)
}

fn read_key(path: &str) -> R<[u8; 32]> {
    let s = std::fs::read_to_string(path)?;
    let b = hex::decode(s.trim())?;
    Ok(<[u8; 32]>::try_from(b).map_err(|_| "key file must hold 32 bytes of hex")?)
}

fn keygen(out: &str) -> R<()> {
    if std::path::Path::new(out).exists() {
        return Err(format!("{out} exists; refusing to overwrite a key").into());
    }
    let mut sk = [0u8; 32];
    getrandom::fill(&mut sk).map_err(|e| e.to_string())?;
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut f = opts.open(out)?;
        writeln!(f, "{}", hex::encode(sk))?;
    }
    println!("wrote {out} (keep it secret)");
    println!("public key (receipt owner): {}", hex::encode(margin_verify::authorship::public_key(&sk)));
    Ok(())
}

fn export(project: &str, module: &str, out: &str, decls: &[String]) -> R<()> {
    let tool = std::env::var("MARGIN_LEAN4EXPORT").unwrap_or_else(|_| "lean4export".into());
    let mut cmd = Command::new("lake");
    cmd.current_dir(project).args(["env", &tool, module, "--"]).args(decls);
    let output = cmd.output().map_err(|e| format!("running `lake env {tool}`: {e}"))?;
    if !output.status.success() {
        return Err(format!("lean4export failed:\n{}", String::from_utf8_lossy(&output.stderr)).into());
    }
    std::fs::write(out, &output.stdout)?;
    eprintln!("exported {module} ({} declarations requested) to {out}", decls.len());
    Ok(())
}

fn prepare(env: &str, project: &str, out: &str, claims: &[String], owner: [u8; 32], open_theorems: bool) -> R<Vec<u8>> {
    let mut arena = Arena::new();
    let env = eb::load_any_env(env, &mut arena)?;
    let proj = eb::read_export(project, &mut arena)?;
    let user = eb::split_user(&env, &arena, proj)?;
    let p = eb::prepare(&env, &arena, &user, claims, owner, open_theorems);
    let bytes = p.input.encode();
    std::fs::write(out, &bytes)?;
    eprintln!(
        "prepared {out}: {} user declarations; environment: {} full + {} statement-only; {} bytes",
        p.user, p.env_full, p.env_header, bytes.len()
    );
    Ok(bytes)
}

fn check(input: &[u8]) -> R<margin_journal::Journal> {
    let input = ProverInput::decode(input).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let j = eb::check(&input)?;
    eprintln!("native pre-check: ACCEPTED ({:.1?})", t.elapsed());
    Ok(j)
}

fn trust(a: &Args) -> R<(Vec<margin_verify::Checker>, Vec<margin_verify::Environment>)> {
    let read = |k: &str, d: &str| -> R<String> {
        Ok(match a.opts.get(k) {
            Some(p) => std::fs::read_to_string(p)?,
            None => d.to_string(),
        })
    };
    Ok((
        margin_verify::load_checkers(&read("--checkers", DEFAULT_CHECKERS)?)?,
        margin_verify::load_environments(&read("--envs", DEFAULT_ENVS)?)?,
    ))
}

fn verify(a: &Args) -> R<margin_verify::Verified> {
    let [receipt, ..] = a.pos.as_slice() else { return Err("missing receipt".into()) };
    let (checkers, envs) = trust(a)?;
    let v = margin_verify::verify_receipt(&std::fs::read(receipt)?, &checkers, &envs)?;
    Ok(v)
}

/// Real environment from a lean4export file shipped with nanoda:
/// `PUnit` and `Wrap : Prop` with `Wrap.mk : Prop → Wrap`.
/// User code: `theorem secret_lemma : Wrap := Wrap.mk Wrap` and `theorem main : Wrap := secret_lemma`.
fn demo_input() -> Vec<u8> {
    const EXPORT: &str = include_str!("../../../third_party/nanoda_lib/test_resources/ProjFromProp/export");
    let mut a = Arena::new();
    let decls = margin_canon::lean4export::read_ndjson(EXPORT, &mut a).expect("bundled export parses");
    let env = &decls[..2];
    let wrap = a.cnst("Wrap", vec![]);
    let mk = a.cnst("Wrap.mk", vec![]);
    let mk_wrap = a.app(mk, wrap);
    let secret = Decl::Theorem { name: Name::from_dotted("secret_lemma"), lparams: vec![], ty: wrap, val: mk_wrap };
    let lemma = a.cnst("secret_lemma", vec![]);
    let main = Decl::Theorem { name: Name::from_dotted("main"), lparams: vec![], ty: wrap, val: lemma };
    let env_set = margin_checker::Env::build(&a, env.to_vec()).expect("valid environment");
    let all: Vec<usize> = (0..env.len()).collect();
    let (env_decls, env_proof) = env_set.open(&a, &all, &[]);
    ProverInput {
        env_root: env_set.tree.root,
        owner: [0; 32],
        claimed: vec![Name::from_dotted("main")],
        env_decls,
        env_proof,
        user_decls: vec![encode_decl(&a, &secret), encode_decl(&a, &main)],
    }
    .encode()
}

fn usage() -> R<()> {
    Err("see `margin` usage at the top of zkvm/host/src/main.rs (or the README)".into())
}

fn run(args: Vec<String>) -> R<()> {
    let Some((cmd, rest)) = args.split_first() else { return usage() };
    let rest = rest.to_vec();
    match cmd.as_str() {
        "keygen" => match rest.as_slice() {
            [out] => keygen(out),
            _ => usage(),
        },
        "export" => {
            let a = parse(rest, &["--project"], &[])?;
            let project = a.opts.get("--project").map(String::as_str).unwrap_or(".");
            match a.pos.as_slice() {
                [module, out, decls @ ..] if !decls.is_empty() => export(project, module, out, decls),
                _ => usage(),
            }
        }
        "env" => match rest.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
            ["cache", src, out] => {
                let env = eb::env_from_export(src, &mut Arena::new())?;
                eb::save_env(&env, out)?;
                println!("wrote {out}\nenv_root: {}", hex::encode(env.tree.root));
                Ok(())
            }
            ["root", p] => {
                let env = eb::load_any_env(p, &mut Arena::new())?;
                println!("declarations: {}\nenv_root: {}", env.decls.len(), hex::encode(env.tree.root));
                Ok(())
            }
            _ => usage(),
        },
        "prepare" => {
            let a = parse(rest, &["--key"], &["--open-theorems"])?;
            let owner = match a.opts.get("--key") {
                Some(k) => margin_verify::authorship::public_key(&read_key(k)?),
                None => [0; 32],
            };
            match a.pos.as_slice() {
                [env, project, out, claims @ ..] if !claims.is_empty() => {
                    prepare(env, project, out, claims, owner, a.switches.contains("--open-theorems")).map(|_| ())
                }
                _ => usage(),
            }
        }
        "check" => match rest.as_slice() {
            [input] => {
                let j = check(&std::fs::read(input)?)?;
                println!("{}", margin_verify::render_journal(&j)?);
                Ok(())
            }
            _ => usage(),
        },
        "execute" => match rest.as_slice() {
            [input] => execute(&std::fs::read(input)?),
            _ => usage(),
        },
        "prove" => match rest.as_slice() {
            [input, out] => prove(&std::fs::read(input)?, out),
            _ => usage(),
        },
        "run" => {
            let a = parse(rest, &["--project", "--key"], &["--open-theorems"])?;
            let [env, module, out, claims @ ..] = a.pos.as_slice() else { return usage() };
            if claims.is_empty() {
                return usage();
            }
            let project = a.opts.get("--project").map(String::as_str).unwrap_or(".");
            let owner = match a.opts.get("--key") {
                Some(k) => margin_verify::authorship::public_key(&read_key(k)?),
                None => {
                    eprintln!("note: no --key, the receipt will be anonymous (no authorship claims possible)");
                    [0; 32]
                }
            };
            let ndjson = format!("{out}.export.ndjson");
            let input_path = format!("{out}.input.bin");
            export(project, module, &ndjson, claims)?;
            let input = prepare(env, &ndjson, &input_path, claims, owner, a.switches.contains("--open-theorems"))?;
            let j = check(&input)?;
            println!("{}", margin_verify::render_journal(&j)?);
            prove(&input, out)
        }
        "verify" => {
            let a = parse(rest, &["--checkers", "--envs", "--claim"], &[])?;
            let v = verify(&a)?;
            println!(
                "VALID: checker {} ({}), environment {}\n\n{}",
                v.checker.name,
                v.checker.status,
                v.environment.name,
                margin_verify::render_journal(&v.journal)?
            );
            if let Some(c) = a.opts.get("--claim") {
                let claim: margin_verify::authorship::Authorship = serde_json::from_str(&std::fs::read_to_string(c)?)?;
                margin_verify::authorship::verify(&v.journal, &claim)?;
                println!("AUTHORSHIP: valid claim by owner {} — note: {:?}", hex::encode(v.journal.owner), claim.note);
            }
            Ok(())
        }
        "match" => {
            let a = parse(rest, &["--checkers", "--envs"], &[])?;
            let [receipt, env, stmt, claims @ ..] = a.pos.as_slice() else { return usage() };
            if claims.is_empty() {
                return usage();
            }
            let (checkers, envs) = trust(&a)?;
            let v = margin_verify::verify_receipt(&std::fs::read(receipt)?, &checkers, &envs)?;
            eprintln!("receipt VALID: checker {}, environment {}", v.checker.name, v.environment.name);
            let mut arena = Arena::new();
            let env = eb::load_any_env(env, &mut arena)?;
            let stmt = eb::read_export(stmt, &mut arena)?;
            println!("{}", eb::match_journal(&v.journal, &env, &mut arena, stmt, claims)?);
            Ok(())
        }
        "claim" => {
            let a = parse(rest, &["--note"], &[])?;
            let [receipt, key] = a.pos.as_slice() else { return usage() };
            let receipt: risc0_zkvm::Receipt = bincode::deserialize(&std::fs::read(receipt)?)?;
            let journal = margin_journal::Journal::decode(&receipt.journal.bytes).map_err(|e| e.to_string())?;
            let note = a.opts.get("--note").cloned().unwrap_or_default();
            let claim = margin_verify::authorship::sign(&read_key(key)?, &journal, &note)?;
            println!("{}", serde_json::to_string_pretty(&claim)?);
            Ok(())
        }
        "image-id" => {
            println!("{}", image_id_hex());
            Ok(())
        }
        "demo" => match rest.as_slice() {
            [out] => prove(&demo_input(), out),
            _ => usage(),
        },
        _ => usage(),
    }
}

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
