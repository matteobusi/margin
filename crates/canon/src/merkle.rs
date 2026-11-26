//! RFC 9162 Merkle trees over environment declarations. See `spec/env_root.md`.

use crate::hash::Hash32;
use crate::wire::{Error, Reader, Writer};
use alloc::vec::Vec;
use sha2::{Digest, Sha256};

/// Environment leaf. `header_hash` commits to the declaration without its proof
/// (equal to `full_hash` for everything except theorems); `axioms_hash` commits to
/// the axioms the declaration depends on, transitively.
pub fn leaf_hash(name_hash: &Hash32, header_hash: &Hash32, full_hash: &Hash32, axioms_hash: &Hash32) -> Hash32 {
    let mut h = Sha256::new();
    h.update([0x00]);
    h.update(name_hash);
    h.update(header_hash);
    h.update(full_hash);
    h.update(axioms_hash);
    h.finalize().into()
}

pub fn node_hash(l: &Hash32, r: &Hash32) -> Hash32 {
    let mut h = Sha256::new();
    h.update([0x01]);
    h.update(l);
    h.update(r);
    h.finalize().into()
}

fn split(n: usize) -> usize {
    // Largest power of two strictly less than n (n >= 2).
    let mut k = 1;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// Merkle tree hash over leaf hashes (RFC 9162 MTH).
pub fn root(leaves: &[Hash32]) -> Hash32 {
    match leaves.len() {
        0 => Sha256::digest([]).into(),
        1 => leaves[0],
        n => {
            let k = split(n);
            node_hash(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// Audit path for leaf `m` (RFC 9162 PATH).
///
/// TODO(perf): O(n) per proof. Precompute subtree roots for large environments.
pub fn path(leaves: &[Hash32], m: usize) -> Vec<Hash32> {
    let n = leaves.len();
    assert!(m < n);
    if n == 1 {
        return Vec::new();
    }
    let k = split(n);
    let mut p;
    if m < k {
        p = path(&leaves[..k], m);
        p.push(root(&leaves[k..]));
    } else {
        p = path(&leaves[k..], m - k);
        p.push(root(&leaves[..k]));
    }
    p
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InclusionProof {
    pub index: u64,
    pub size: u64,
    pub path: Vec<Hash32>,
}

impl InclusionProof {
    /// Verify that `leaf` is at `index` in a tree of `size` leaves with root `root`
    /// (RFC 9162 section 2.1.3.2).
    pub fn verify(&self, leaf: &Hash32, root: &Hash32) -> bool {
        if self.index >= self.size {
            return false;
        }
        let (mut fnn, mut sn) = (self.index, self.size - 1);
        let mut r = *leaf;
        for p in &self.path {
            if sn == 0 {
                return false;
            }
            if fnn & 1 == 1 || fnn == sn {
                r = node_hash(p, &r);
                while fnn & 1 == 0 && fnn != 0 {
                    fnn >>= 1;
                    sn >>= 1;
                }
            } else {
                r = node_hash(&r, p);
            }
            fnn >>= 1;
            sn >>= 1;
        }
        sn == 0 && r == *root
    }

    pub fn encode(&self, w: &mut Writer) {
        w.uvar(self.index);
        w.uvar(self.size);
        w.uvar(self.path.len() as u64);
        for h in &self.path {
            w.raw(h);
        }
    }

    pub fn decode(r: &mut Reader) -> Result<Self, Error> {
        let index = r.uvar()?;
        let size = r.uvar()?;
        let n = r.uvar()?;
        if n > 64 {
            return Err(Error::Overflow);
        }
        let path = (0..n).map(|_| r.array32()).collect::<Result<_, _>>()?;
        Ok(Self { index, size, path })
    }
}

/// One proof that several leaves are in the tree (RFC 9162 tree shape).
///
/// `hashes` are the roots of the maximal subtrees that contain none of the proven
/// leaves, in the order a left-to-right recursion over the tree meets them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiProof {
    pub size: u64,
    /// Leaf indices, strictly increasing.
    pub indices: Vec<u64>,
    pub hashes: Vec<Hash32>,
}

impl MultiProof {
    /// Verify that `leaves[i]` is the leaf at `indices[i]`, for all `i`, under `root`.
    pub fn verify(&self, leaves: &[Hash32], root: &Hash32) -> bool {
        if leaves.len() != self.indices.len() || self.size == 0 {
            return false;
        }
        if self.indices.windows(2).any(|w| w[0] >= w[1]) || self.indices.last().is_some_and(|&i| i >= self.size) {
            return false;
        }
        let mut next_hash = 0usize;
        let mut next_leaf = 0usize;
        let r = self.recompute(0, self.size, leaves, &mut next_leaf, &mut next_hash);
        matches!(r, Some(h) if h == *root) && next_hash == self.hashes.len() && next_leaf == leaves.len()
    }

    fn recompute(&self, lo: u64, hi: u64, leaves: &[Hash32], nl: &mut usize, nh: &mut usize) -> Option<Hash32> {
        let has_leaf = self.indices.get(*nl).is_some_and(|&i| i < hi);
        if !has_leaf {
            let h = *self.hashes.get(*nh)?;
            *nh += 1;
            return Some(h);
        }
        if hi - lo == 1 {
            let h = leaves[*nl];
            *nl += 1;
            return Some(h);
        }
        let k = split((hi - lo) as usize) as u64;
        let l = self.recompute(lo, lo + k, leaves, nl, nh)?;
        let r = self.recompute(lo + k, hi, leaves, nl, nh)?;
        Some(node_hash(&l, &r))
    }

    pub fn encode(&self, w: &mut Writer) {
        w.uvar(self.size);
        w.uvar(self.indices.len() as u64);
        for i in &self.indices {
            w.uvar(*i);
        }
        w.uvar(self.hashes.len() as u64);
        for h in &self.hashes {
            w.raw(h);
        }
    }

    pub fn decode(r: &mut Reader) -> Result<Self, Error> {
        let size = r.uvar()?;
        let n = r.count()?;
        let indices = (0..n).map(|_| r.uvar()).collect::<Result<_, _>>()?;
        let n = r.count()?;
        let hashes = (0..n).map(|_| r.array32()).collect::<Result<_, _>>()?;
        Ok(Self { size, indices, hashes })
    }
}

/// An environment tree: leaves sorted by name hash.
pub struct EnvTree {
    /// `(name_hash, leaf_hash)`, sorted by name hash.
    pub entries: Vec<(Hash32, Hash32)>,
    pub leaves: Vec<Hash32>,
    pub root: Hash32,
}

impl EnvTree {
    /// Build from `(name_hash, leaf_hash)` pairs. Returns `None` on duplicate keys.
    pub fn build(mut entries: Vec<(Hash32, Hash32)>) -> Option<Self> {
        entries.sort();
        if entries.windows(2).any(|w| w[0].0 == w[1].0) {
            return None;
        }
        let leaves: Vec<Hash32> = entries.iter().map(|(_, l)| *l).collect();
        let root = root(&leaves);
        Some(Self { entries, leaves, root })
    }

    pub fn find(&self, name_hash: &Hash32) -> Option<usize> {
        self.entries.binary_search_by(|(n, _)| n.cmp(name_hash)).ok()
    }

    /// Multi-proof for the given leaf indices (any order; duplicates removed).
    pub fn multiproof(&self, indices: &[usize]) -> MultiProof {
        let mut idx: Vec<usize> = indices.to_vec();
        idx.sort_unstable();
        idx.dedup();
        let mut hashes = Vec::new();
        let mut pos = 0usize;
        fn go(leaves: &[Hash32], lo: usize, idx: &[usize], pos: &mut usize, out: &mut Vec<Hash32>) {
            let hi = lo + leaves.len();
            if !(*pos < idx.len() && idx[*pos] < hi) {
                out.push(root(leaves));
                return;
            }
            if leaves.len() == 1 {
                *pos += 1;
                return;
            }
            let k = split(leaves.len());
            go(&leaves[..k], lo, idx, pos, out);
            go(&leaves[k..], lo + k, idx, pos, out);
        }
        go(&self.leaves, 0, &idx, &mut pos, &mut hashes);
        MultiProof { size: self.leaves.len() as u64, indices: idx.iter().map(|&i| i as u64).collect(), hashes }
    }

    pub fn prove(&self, index: usize) -> InclusionProof {
        InclusionProof {
            index: index as u64,
            size: self.leaves.len() as u64,
            path: path(&self.leaves, index),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(i: u8) -> Hash32 {
        [i; 32]
    }

    #[test]
    fn every_leaf_verifies_for_many_sizes() {
        for n in 1..=33u8 {
            let tree = EnvTree::build((0..n).map(|i| (h(i), h(i ^ 0xaa))).collect()).unwrap();
            for i in 0..n as usize {
                let p = tree.prove(i);
                assert!(p.verify(&tree.leaves[i], &tree.root), "n={n} i={i}");
                // Wrong leaf fails.
                assert!(!p.verify(&h(0xff), &tree.root));
                // Wrong index fails (when there is more than one leaf).
                if n > 1 {
                    let bad = InclusionProof { index: (i as u64 + 1) % n as u64, ..p.clone() };
                    assert!(!bad.verify(&tree.leaves[i], &tree.root), "n={n} i={i}");
                }
            }
        }
    }

    #[test]
    fn multiproofs_verify_and_reject_tampering() {
        for n in 1..=40u8 {
            let tree = EnvTree::build((0..n).map(|i| (h(i), h(i ^ 0x55))).collect()).unwrap();
            let n = n as usize;
            for pick in [vec![0], vec![n - 1], (0..n).step_by(3).collect::<Vec<_>>(), (0..n).collect()] {
                let p = tree.multiproof(&pick);
                let leaves: Vec<Hash32> = p.indices.iter().map(|&i| tree.leaves[i as usize]).collect();
                assert!(p.verify(&leaves, &tree.root), "n={n} pick={pick:?}");
                // Wrong leaf.
                let mut bad = leaves.clone();
                bad[0][0] ^= 1;
                assert!(!p.verify(&bad, &tree.root));
                // Shifted index.
                if n > 1 && p.indices.len() < n {
                    let mut q = p.clone();
                    let last = q.indices.len() - 1;
                    q.indices[last] = (q.indices[last] + 1) % n as u64;
                    if q.indices.windows(2).all(|w| w[0] < w[1]) {
                        assert!(!q.verify(&leaves, &tree.root), "n={n} pick={pick:?}");
                    }
                }
                // Extra or missing hash.
                let mut q = p.clone();
                q.hashes.push([0; 32]);
                assert!(!q.verify(&leaves, &tree.root));
            }
        }
    }

    #[test]
    fn duplicate_keys_rejected() {
        assert!(EnvTree::build(alloc::vec![(h(1), h(2)), (h(1), h(3))]).is_none());
    }
}
