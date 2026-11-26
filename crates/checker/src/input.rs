use alloc::vec::Vec;
use crate::env::EnvEntry;
use margin_canon::merkle::MultiProof;
use margin_canon::wire::{Error, Reader, Writer};
use margin_canon::{Hash32, Name};

const MAGIC: &[u8; 4] = b"INP3";

/// Everything the guest reads. All of it is private except what goes to the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProverInput {
    pub env_root: Hash32,
    pub owner: [u8; 32],
    /// Names of the theorems to claim.
    pub claimed: Vec<Name>,
    /// Environment declarations that the user declarations need, in leaf order (the
    /// order of `env_proof.indices`).
    pub env_decls: Vec<EnvEntry>,
    /// One Merkle multi-proof for all of `env_decls`.
    pub env_proof: MultiProof,
    /// CE bytes of user declarations, in dependency order.
    pub user_decls: Vec<Vec<u8>>,
}

impl ProverInput {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.raw(&self.env_root);
        w.raw(&self.owner);
        w.names(&self.claimed);
        w.uvar(self.env_decls.len() as u64);
        for e in &self.env_decls {
            e.encode(&mut w);
        }
        self.env_proof.encode(&mut w);
        w.uvar(self.user_decls.len() as u64);
        for d in &self.user_decls {
            w.bytes(d);
        }
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        if r.raw(4)? != MAGIC {
            return Err(Error::BadMagic);
        }
        let env_root = r.array32()?;
        let owner = r.array32()?;
        let claimed = r.names()?;
        let n = r.count()?;
        let env_decls = (0..n).map(|_| EnvEntry::decode(&mut r)).collect::<Result<_, _>>()?;
        let env_proof = MultiProof::decode(&mut r)?;
        let n = r.count()?;
        let user_decls = (0..n).map(|_| r.bytes().map(<[u8]>::to_vec)).collect::<Result<_, _>>()?;
        r.finish()?;
        Ok(Self { env_root, owner, claimed, env_decls, env_proof, user_decls })
    }
}
