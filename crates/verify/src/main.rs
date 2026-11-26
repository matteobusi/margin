//! `margin-verify` — verify a receipt, or inspect a raw journal.
//!
//! ```text
//! margin-verify receipt <receipt.bin> [--checkers checkers.json] [--envs envs.json]   (needs --features risc0)
//! margin-verify journal <journal.bin>                              (no verification)
//! ```

use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage:\n  margin-verify receipt <receipt.bin> [--checkers checkers.json] [--envs envs.json]\n  margin-verify journal <journal.bin>");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("journal") if args.len() == 2 => journal(&args[1]),
        Some("receipt") if args.len() >= 2 => {
            let (mut checkers, mut envs) = ("checkers.json".to_string(), "envs.json".to_string());
            let mut rest = args[2..].iter();
            while let Some(flag) = rest.next() {
                match (flag.as_str(), rest.next()) {
                    ("--checkers", Some(v)) => checkers = v.clone(),
                    ("--envs", Some(v)) => envs = v.clone(),
                    _ => return usage(),
                }
            }
            receipt(&args[1], &checkers, &envs)
        }
        _ => usage(),
    }
}

fn journal(path: &str) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match margin_journal::Journal::decode(&bytes).map_err(|e| e.to_string()).and_then(|j| {
        margin_verify::render_journal(&j).map_err(|e| e.to_string())
    }) {
        Ok(s) => {
            println!("NOT VERIFIED — journal contents only\n\n{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "risc0")]
fn receipt(path: &str, checkers: &str, envs: &str) -> ExitCode {
    let run = || -> Result<String, Box<dyn std::error::Error>> {
        let accepted = margin_verify::load_checkers(&std::fs::read_to_string(checkers)?)?;
        let envs = margin_verify::load_environments(&std::fs::read_to_string(envs)?)?;
        let v = margin_verify::verify_receipt(&std::fs::read(path)?, &accepted, &envs)?;
        Ok(format!(
            "VALID — checker {} ({}), environment {}\n\n{}",
            v.checker.name,
            v.checker.status,
            v.environment.name,
            margin_verify::render_journal(&v.journal)?
        ))
    };
    match run() {
        Ok(s) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("INVALID: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "risc0"))]
fn receipt(_: &str, _: &str, _: &str) -> ExitCode {
    eprintln!("error: built without receipt verification. Rebuild with `--features risc0`.");
    ExitCode::FAILURE
}
