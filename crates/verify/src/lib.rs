//! Verify margin receipts and render their journal for humans.

pub mod authorship;
mod pretty;

pub use pretty::render_decl;

use margin_canon::{decode_decl, Arena};
use margin_journal::Journal;
use serde::{Deserialize, Serialize};

/// One accepted checker: a guest image ID that the verifier trusts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Checker {
    pub name: String,
    /// RISC Zero image ID, 32 bytes, hex.
    pub image_id: String,
    /// `experimental`, `trusted`, ... (informational).
    #[serde(default)]
    pub status: String,
}

pub fn load_checkers(json: &str) -> Result<Vec<Checker>, serde_json::Error> {
    serde_json::from_str(json)
}

/// One accepted environment: an `env_root` that the verifier trusts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Environment {
    /// For example `Lean v4.36.0 Init`.
    pub name: String,
    /// 32 bytes, hex.
    pub env_root: String,
    /// How to rebuild the root (informational).
    #[serde(default)]
    pub rebuild: String,
}

pub fn load_environments(json: &str) -> Result<Vec<Environment>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Check the journal's `env_root` against the accepted environments. Without this, a
/// prover could use a fake library (for example one where `Nat.Prime := True`).
pub fn accepted_environment<'a>(journal: &Journal, accepted: &'a [Environment]) -> Result<&'a Environment, VerifyError> {
    let root = hex::encode(journal.env_root);
    accepted
        .iter()
        .find(|e| e.env_root.eq_ignore_ascii_case(&root))
        .ok_or(VerifyError::UnknownEnvironment(root))
}

#[derive(Debug)]
pub enum VerifyError {
    Journal(margin_canon::Error),
    Receipt(String),
    NotSuccinct,
    NoAcceptedChecker,
    UnknownEnvironment(String),
    Authorship(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Journal(e) => write!(f, "bad journal: {e}"),
            VerifyError::Receipt(e) => write!(f, "bad receipt: {e}"),
            VerifyError::NotSuccinct => write!(f, "receipt is not succinct (it can leak proof size)"),
            VerifyError::NoAcceptedChecker => write!(f, "receipt does not verify against any accepted checker"),
            VerifyError::UnknownEnvironment(r) => write!(f, "env_root {r} is not an accepted environment"),
            VerifyError::Authorship(m) => write!(f, "authorship claim rejected: {m}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// A verified claim set.
#[derive(Debug)]
pub struct Verified {
    pub checker: Checker,
    pub environment: Environment,
    pub journal: Journal,
}

/// Verify a serialized RISC Zero receipt against the accepted checkers and environments.
#[cfg(feature = "risc0")]
pub fn verify_receipt(receipt: &[u8], accepted: &[Checker], envs: &[Environment]) -> Result<Verified, VerifyError> {
    use risc0_zkvm::{sha::Digest, InnerReceipt, Receipt};
    let receipt: Receipt =
        bincode::deserialize(receipt).map_err(|e| VerifyError::Receipt(e.to_string()))?;
    // Composite receipts reveal the segment count. Only accept succinct ones.
    if !matches!(receipt.inner, InnerReceipt::Succinct(_)) {
        return Err(VerifyError::NotSuccinct);
    }
    for c in accepted {
        let Ok(bytes) = hex::decode(&c.image_id) else { continue };
        let Ok(id) = Digest::try_from(bytes.as_slice()) else { continue };
        if receipt.verify(id).is_ok() {
            let journal = Journal::decode(&receipt.journal.bytes).map_err(VerifyError::Journal)?;
            let environment = accepted_environment(&journal, envs)?.clone();
            return Ok(Verified { checker: c.clone(), environment, journal });
        }
    }
    Err(VerifyError::NoAcceptedChecker)
}

/// Human-readable report of a journal. Does NOT verify anything by itself.
pub fn render_journal(j: &Journal) -> Result<String, margin_canon::Error> {
    use std::fmt::Write;
    let mut out = String::new();
    let mut arena = Arena::new();
    writeln!(out, "checker:  {}", j.checker).unwrap();
    writeln!(out, "env_root: {}", hex::encode(j.env_root)).unwrap();
    if j.owner == [0; 32] {
        writeln!(out, "owner:    none (anonymous receipt, authorship cannot be claimed)").unwrap();
    } else {
        writeln!(out, "owner:    {} (Ed25519 public key)", hex::encode(j.owner)).unwrap();
    }
    let axioms: Vec<String> = j.axioms.iter().map(|a| a.to_string()).collect();
    writeln!(out, "axioms:   [{}]", axioms.join(", ")).unwrap();
    writeln!(out, "\nclaims:").unwrap();
    for c in &j.claims {
        let d = decode_decl(c, &mut arena)?;
        writeln!(out, "  {}", render_decl(&arena, &d)).unwrap();
    }
    if !j.stmt_cone.is_empty() {
        writeln!(out, "\nstatement dependencies (public):").unwrap();
        for c in &j.stmt_cone {
            let d = decode_decl(c, &mut arena)?;
            writeln!(out, "  {}", render_decl(&arena, &d)).unwrap();
        }
    }
    Ok(out)
}
