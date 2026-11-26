//! `margin-envbuild` — environment roots and caches, prover inputs, native pre-check,
//! statement matching. (The `margin` CLI offers the same commands plus proving.)
//!
//! ```text
//! margin-envbuild root    <env.ndjson|env.cache>
//! margin-envbuild cache   <env.ndjson> <out.cache>
//! margin-envbuild prepare [--reachable | --open-theorems] [--owner HEX] <env> <project.ndjson> <input.bin> <claim>...
//! margin-envbuild check   <input.bin> [--journal-out journal.bin]
//! margin-envbuild match   <receipt.bin> <checkers.json> <envs.json> <env> <statement.ndjson> <claim>...
//! margin-envbuild match-journal <journal.bin> <env> <statement.ndjson> <claim>...
//! ```
//!
//! `*.ndjson` files are lean4export (format 3.1) outputs. An environment is either a
//! lean4export file of the trusted library (for example `lean4export Mathlib`) or a cache
//! built from one with `cache`. By default, `prepare` opens in full only the library
//! declarations the kernel consults (found by a recorded native run) and everything else
//! as a statement. `--reachable` opens everything reachable (theorems as statements);
//! `--open-theorems` also opens theorems in full.

use margin_canon::Arena;
use margin_envbuild::*;
use margin_journal::Journal;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |f: &str| args.iter().position(|a| a == f).map(|p| args.remove(p)).is_some();
    let mode = match (flag("--open-theorems"), flag("--reachable")) {
        (true, _) => OpenMode::Reachable { open_theorems: true },
        (false, true) => OpenMode::Reachable { open_theorems: false },
        _ => OpenMode::Minimal,
    };
    let owner = match args.iter().position(|a| a == "--owner") {
        Some(p) if p + 1 < args.len() => {
            let hexkey = args.remove(p + 1);
            args.remove(p);
            match hex::decode(&hexkey).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()) {
                Some(k) => k,
                None => {
                    eprintln!("error: --owner expects a 32-byte hex public key");
                    return ExitCode::from(2);
                }
            }
        }
        _ => [0; 32],
    };
    let strs: Vec<&str> = args.iter().map(String::as_str).collect();
    let claims_of = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let res: R<()> = match strs.as_slice() {
        ["root", p] => load_any_env(p, &mut Arena::new()).map(|env| {
            println!("declarations: {}", env.decls.len());
            println!("env_root:     {}", hex::encode(env.tree.root));
        }),
        ["cache", src, out] => {
            let env = env_from_export(src, &mut Arena::new());
            env.and_then(|env| {
                save_env(&env, out)?;
                println!("wrote {out}\nenv_root: {}", hex::encode(env.tree.root));
                Ok(())
            })
        }
        ["prepare", env, proj, out, claims @ ..] if !claims.is_empty() => (|| {
            let mut arena = Arena::new();
            let env = load_any_env(env, &mut arena)?;
            let proj = read_export(proj, &mut arena)?;
            let user = split_user(&env, &arena, proj)?;
            let p = prepare(&env, &arena, &user, &claims_of(claims), owner, mode)?;
            let bytes = p.input.encode();
            std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
            eprintln!(
                "wrote {out}: {} user declarations; environment: {} full + {} statement-only; {} bytes",
                p.user, p.env_full, p.env_header, bytes.len()
            );
            Ok(())
        })(),
        ["check", p, rest @ ..] => (|| {
            let bytes = std::fs::read(p).map_err(|e| format!("{p}: {e}"))?;
            let input = margin_checker::ProverInput::decode(&bytes).map_err(|e| e.to_string())?;
            let journal = check(&input)?;
            if let ["--journal-out", j] = rest {
                std::fs::write(j, journal.encode()).map_err(|e| format!("{j}: {e}"))?;
            }
            let report = margin_verify::render_journal(&journal).map_err(|e| e.to_string())?;
            println!("ACCEPTED (native pre-check, no proof)\n\n{report}");
            Ok(())
        })(),
        ["match", receipt, checkers, envs, env, stmt, claims @ ..] if !claims.is_empty() => (|| {
            let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
            let accepted = margin_verify::load_checkers(&read(checkers)?).map_err(|e| e.to_string())?;
            let envs = margin_verify::load_environments(&read(envs)?).map_err(|e| e.to_string())?;
            let bytes = std::fs::read(receipt).map_err(|e| format!("{receipt}: {e}"))?;
            let v = margin_verify::verify_receipt(&bytes, &accepted, &envs).map_err(|e| format!("INVALID: {e}"))?;
            eprintln!("receipt VALID: checker {}, environment {}", v.checker.name, v.environment.name);
            let mut arena = Arena::new();
            let env = load_any_env(env, &mut arena)?;
            let stmt = read_export(stmt, &mut arena)?;
            println!("{}", match_journal(&v.journal, &env, &mut arena, stmt, &claims_of(claims))?);
            Ok(())
        })(),
        ["match-journal", j, env, stmt, claims @ ..] if !claims.is_empty() => (|| {
            let b = std::fs::read(j).map_err(|e| format!("{j}: {e}"))?;
            let journal = Journal::decode(&b).map_err(|e| e.to_string())?;
            eprintln!("NOTE: journal only, no receipt verification");
            let mut arena = Arena::new();
            let env = load_any_env(env, &mut arena)?;
            let stmt = read_export(stmt, &mut arena)?;
            println!("{}", match_journal(&journal, &env, &mut arena, stmt, &claims_of(claims))?);
            Ok(())
        })(),
        _ => Err("usage: see the header of crates/envbuild/src/main.rs".into()),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
