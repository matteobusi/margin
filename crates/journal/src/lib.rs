//! The public journal. See `spec/journal.md`.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

use alloc::{string::String, vec::Vec};
use margin_canon::wire::{Error, Reader, Writer};
use margin_canon::{Hash32, Name};

/// Version 1 (`JRN1`): single-run receipts (checker v0.0.1).
pub const MAGIC_V1: &[u8; 4] = b"JRN1";
/// Version 2 (`JRN2`): adds `assumed_id` (receipts composed from module receipts).
pub const MAGIC: &[u8; 4] = b"JRN2";
pub const VERSION: u64 = 2;

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
    /// Version 2: image ID that every assumed module receipt was verified against, or
    /// all zeros if the run assumed none. A verifier must require it to equal the image
    /// ID of the receipt itself (see `spec/journal.md`).
    pub assumed_id: Hash32,
}

impl Journal {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(if self.version == 1 { MAGIC_V1 } else { MAGIC });
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
        if self.version >= 2 {
            w.raw(&self.assumed_id);
        }
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let expected = match r.raw(4)? {
            m if m == MAGIC_V1 => 1,
            m if m == MAGIC => 2,
            _ => return Err(Error::BadMagic),
        };
        let version = r.uvar()?;
        if version != expected {
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
        let assumed_id = if version >= 2 { r.array32()? } else { [0; 32] };
        r.finish()?;
        let j = Journal { version, checker, env_root, owner, claims, stmt_cone, axioms, assumed_id };
        if j.encode() != bytes {
            return Err(Error::NonCanonical);
        }
        Ok(j)
    }
}

/// Magic of a module journal.
pub const MOD_MAGIC: &[u8; 4] = b"MOD1";

/// The journal of a module receipt: "the declarations whose interface is `iface_root`
/// type-check over `env_root`, assuming module receipts of image `self_id`". Module
/// receipts are intermediate: a verifier never accepts one as a final receipt (the magic
/// differs from [`Journal`]'s), they are only assumed by other runs of the same image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModJournal {
    /// Image ID this run required of the module receipts it assumed (and that runs
    /// assuming this one must use).
    pub self_id: Hash32,
    pub env_root: Hash32,
    /// Lean module name (informational).
    pub module: Name,
    /// Merkle root of the module's interface (same leaf format as the environment).
    pub iface_root: Hash32,
    /// Number of leaves of the interface tree.
    pub iface_size: u64,
    /// Interface roots of the module receipts this run assumed (informational).
    pub deps: Vec<Hash32>,
}

impl ModJournal {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MOD_MAGIC);
        w.raw(&self.self_id);
        w.raw(&self.env_root);
        w.name(&self.module);
        w.raw(&self.iface_root);
        w.uvar(self.iface_size);
        w.uvar(self.deps.len() as u64);
        for d in &self.deps {
            w.raw(d);
        }
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.raw(4)? != MOD_MAGIC {
            return Err(Error::BadMagic);
        }
        let self_id = r.array32()?;
        let env_root = r.array32()?;
        let module = r.name()?;
        let iface_root = r.array32()?;
        let iface_size = r.uvar()?;
        let n = r.count()?;
        let deps = (0..n).map(|_| r.array32()).collect::<Result<_, _>>()?;
        r.finish()?;
        let j = ModJournal { self_id, env_root, module, iface_root, iface_size, deps };
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
            assumed_id: [3; 32],
        };
        let b = j.encode();
        assert_eq!(Journal::decode(&b).unwrap(), j);
        let mut bad = b.clone();
        bad.push(0);
        assert!(Journal::decode(&bad).is_err());
        // Version 1 still decodes (without assumed_id).
        let v1 = Journal { version: 1, assumed_id: [0; 32], ..j };
        assert_eq!(Journal::decode(&v1.encode()).unwrap(), v1);
    }

    #[test]
    fn module_roundtrip() {
        let m = ModJournal {
            self_id: [1; 32],
            env_root: [2; 32],
            module: Name::from_dotted("Lib.Foo"),
            iface_root: [4; 32],
            iface_size: 17,
            deps: vec![[5; 32], [6; 32]],
        };
        let b = m.encode();
        assert_eq!(ModJournal::decode(&b).unwrap(), m);
        assert!(Journal::decode(&b).is_err(), "a module journal is never a final journal");
    }
}
