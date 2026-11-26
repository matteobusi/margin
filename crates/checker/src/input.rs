use alloc::vec::Vec;
use crate::env::EnvEntry;
use margin_canon::merkle::MultiProof;
use margin_canon::wire::{Error, Reader, Writer};
use margin_canon::{Hash32, Name};

const MAGIC: &[u8; 4] = b"INP5";

/// Declarations opened from an assumed module receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DepOpening {
    /// The module journal (`MOD1`) of the assumed receipt.
    pub journal: Vec<u8>,
    /// Interface entries, in leaf order (the order of `proof.indices`).
    pub entries: Vec<EnvEntry>,
    /// One multi-proof of `entries` against the journal's `iface_root`.
    pub proof: MultiProof,
}

impl DepOpening {
    fn encode(&self, w: &mut Writer) {
        w.bytes(&self.journal);
        w.uvar(self.entries.len() as u64);
        for e in &self.entries {
            e.encode(w);
        }
        self.proof.encode(w);
    }

    fn decode(r: &mut Reader) -> Result<Self, Error> {
        let journal = r.bytes()?.to_vec();
        let n = r.count()?;
        let entries = (0..n).map(|_| EnvEntry::decode(r)).collect::<Result<_, _>>()?;
        let proof = MultiProof::decode(r)?;
        Ok(Self { journal, entries, proof })
    }
}

/// Everything the guest reads. All of it is private except what goes to the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProverInput {
    pub env_root: Hash32,
    pub owner: [u8; 32],
    /// Image ID that assumed module receipts must have (and that this run's module
    /// journal records). All zeros when `deps` is empty and no module journal is made.
    pub self_id: Hash32,
    /// Lean module name; recorded in the module journal.
    pub module: Name,
    /// Names of the theorems to claim. Empty: module mode (the run commits a module
    /// journal instead of the public one).
    pub claimed: Vec<Name>,
    /// Environment declarations that the user declarations need, in leaf order (the
    /// order of `env_proof.indices`).
    pub env_decls: Vec<EnvEntry>,
    /// One Merkle multi-proof for all of `env_decls`.
    pub env_proof: MultiProof,
    /// Assumed module receipts and what this run opens from them.
    pub deps: Vec<DepOpening>,
    /// CE bytes of user declarations, in dependency order.
    pub user_decls: Vec<Vec<u8>>,
}

impl ProverInput {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.raw(&self.env_root);
        w.raw(&self.owner);
        w.raw(&self.self_id);
        w.name(&self.module);
        w.names(&self.claimed);
        w.uvar(self.env_decls.len() as u64);
        for e in &self.env_decls {
            e.encode(&mut w);
        }
        self.env_proof.encode(&mut w);
        w.uvar(self.deps.len() as u64);
        for d in &self.deps {
            d.encode(&mut w);
        }
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
        let self_id = r.array32()?;
        let module = r.name()?;
        let claimed = r.names()?;
        let n = r.count()?;
        let env_decls = (0..n).map(|_| EnvEntry::decode(&mut r)).collect::<Result<_, _>>()?;
        let env_proof = MultiProof::decode(&mut r)?;
        let n = r.count()?;
        let deps = (0..n).map(|_| DepOpening::decode(&mut r)).collect::<Result<_, _>>()?;
        let n = r.count()?;
        let user_decls = (0..n).map(|_| r.bytes().map(<[u8]>::to_vec)).collect::<Result<_, _>>()?;
        r.finish()?;
        Ok(Self { env_root, owner, self_id, module, claimed, env_decls, env_proof, deps, user_decls })
    }
}
