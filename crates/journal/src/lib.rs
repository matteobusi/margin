//! The public journal. See `spec/journal.md`.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::{string::String, vec::Vec};
use margin_canon::wire::{Error, Reader, Writer};
use margin_canon::{Hash32, Name};

pub const MAGIC: &[u8; 4] = b"JRN1";
pub const VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    pub version: u64,
    /// Name and version of the checker. Informational only: the verifier binds the
    /// checker through the image ID it verifies the receipt against.
    pub checker: String,
    pub env_root: Hash32,
    /// Ed25519 public key of the prover.
    pub owner: [u8; 32],
    /// CE `Statement` declarations of the claimed theorems.
    pub claims: Vec<Vec<u8>>,
    /// CE declarations that the statements depend on, outside the environment.
    pub stmt_cone: Vec<Vec<u8>>,
    /// Axioms that the claims use, sorted.
    pub axioms: Vec<Name>,
}

impl Journal {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.uvar(self.version);
        w.str(&self.checker);
        w.raw(&self.env_root);
        w.raw(&self.owner);
        w.uvar(self.claims.len() as u64);
        for c in &self.claims {
            w.bytes(c);
        }
        w.uvar(self.stmt_cone.len() as u64);
        for c in &self.stmt_cone {
            w.bytes(c);
        }
        w.names(&self.axioms);
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.raw(4)? != MAGIC {
            return Err(Error::BadMagic);
        }
        let version = r.uvar()?;
        if version != VERSION {
            return Err(Error::BadTag(version as u8));
        }
        let checker = r.str()?;
        let env_root = r.array32()?;
        let owner = r.array32()?;
        let n = r.count()?;
        let claims = (0..n).map(|_| r.bytes().map(<[u8]>::to_vec)).collect::<Result<_, _>>()?;
        let n = r.count()?;
        let stmt_cone =
            (0..n).map(|_| r.bytes().map(<[u8]>::to_vec)).collect::<Result<_, _>>()?;
        let axioms = r.names()?;
        r.finish()?;
        let j = Journal { version, checker, env_root, owner, claims, stmt_cone, axioms };
        if j.encode() != bytes {
            return Err(Error::NonCanonical);
        }
        Ok(j)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn roundtrip() {
        let j = Journal {
            version: VERSION,
            checker: "test-checker 0.0.1".into(),
            env_root: [7; 32],
            owner: [9; 32],
            claims: vec![vec![1, 2, 3]],
            stmt_cone: vec![],
            axioms: vec![Name::from_dotted("propext")],
        };
        let b = j.encode();
        assert_eq!(Journal::decode(&b).unwrap(), j);
        let mut bad = b.clone();
        bad.push(0);
        assert!(Journal::decode(&bad).is_err());
    }
}
