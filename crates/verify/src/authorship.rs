//! Authorship claims.
//!
//! A receipt's journal carries an `owner`: an Ed25519 public key chosen by the prover.
//! It is part of the proven statement, so it cannot be changed after proving. Anyone can
//! copy a receipt, but only the holder of the matching secret key can sign an
//! authorship claim for it. The key reveals nothing about who the prover is, unless the
//! prover chooses to link it to an identity (for example with a signed `note`).

use crate::VerifyError;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use margin_journal::Journal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain separation for authorship signatures.
pub const TAG: &[u8] = b"ce1/authorship/v1";

/// A signed authorship claim for one receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authorship {
    /// SHA-256 of the journal bytes, hex.
    pub journal_sha256: String,
    /// Free text signed together with the claim (for example "this is Alice").
    #[serde(default)]
    pub note: String,
    /// Ed25519 signature, hex.
    pub signature: String,
}

pub fn journal_sha256(j: &Journal) -> [u8; 32] {
    Sha256::digest(j.encode()).into()
}

fn message(journal_hash: &[u8; 32], note: &str) -> Vec<u8> {
    let mut m = Vec::with_capacity(TAG.len() + 32 + 8 + note.len());
    m.extend_from_slice(TAG);
    m.extend_from_slice(journal_hash);
    m.extend_from_slice(&(note.len() as u64).to_le_bytes());
    m.extend_from_slice(note.as_bytes());
    m
}

/// Public key (the journal `owner`) for a 32-byte secret key.
pub fn public_key(secret: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// Sign an authorship claim. Fails if `secret` does not belong to the journal's owner.
pub fn sign(secret: &[u8; 32], journal: &Journal, note: &str) -> Result<Authorship, VerifyError> {
    let key = SigningKey::from_bytes(secret);
    if key.verifying_key().to_bytes() != journal.owner {
        return Err(VerifyError::Authorship("this key is not the receipt's owner".into()));
    }
    let h = journal_sha256(journal);
    let sig = key.sign(&message(&h, note));
    Ok(Authorship { journal_sha256: hex::encode(h), note: note.into(), signature: hex::encode(sig.to_bytes()) })
}

/// Check an authorship claim against a journal (which must come from a verified receipt).
pub fn verify(journal: &Journal, a: &Authorship) -> Result<(), VerifyError> {
    let bad = |m: &str| Err(VerifyError::Authorship(m.into()));
    if journal.owner == [0; 32] {
        return bad("the receipt has no owner key");
    }
    let h = journal_sha256(journal);
    if !a.journal_sha256.eq_ignore_ascii_case(&hex::encode(h)) {
        return bad("the claim is for a different receipt");
    }
    let Ok(sig_bytes) = hex::decode(&a.signature) else { return bad("signature is not hex") };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else { return bad("malformed signature") };
    let Ok(vk) = VerifyingKey::from_bytes(&journal.owner) else { return bad("owner is not a valid Ed25519 key") };
    vk.verify_strict(&message(&h, &a.note), &sig).or_else(|_| bad("signature does not verify"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(owner: [u8; 32]) -> Journal {
        Journal {
            version: margin_journal::VERSION,
            checker: "test".into(),
            env_root: [1; 32],
            owner,
            claims: vec![vec![1, 2, 3]],
            stmt_cone: vec![],
            axioms: vec![],
            assumed_id: [0; 32],
        }
    }

    #[test]
    fn sign_and_verify() {
        let sk = [7u8; 32];
        let j = journal(public_key(&sk));
        let a = sign(&sk, &j, "I wrote this").unwrap();
        verify(&j, &a).unwrap();
        // Tampered note, other receipt, wrong key, no owner.
        assert!(verify(&j, &Authorship { note: "someone else".into(), ..a.clone() }).is_err());
        let mut j2 = j.clone();
        j2.claims[0][0] = 9;
        assert!(verify(&j2, &a).is_err());
        assert!(sign(&[8u8; 32], &j, "").is_err());
        assert!(verify(&journal([0; 32]), &a).is_err());
    }
}
