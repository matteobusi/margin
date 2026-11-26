use crate::types::Name;
use crate::wire::Writer;
use sha2::{Digest, Sha256};

pub type Hash32 = [u8; 32];

fn tagged(tag: &[u8], data: &[u8]) -> Hash32 {
    let mut h = Sha256::new();
    h.update(tag);
    h.update(data);
    h.finalize().into()
}

/// `SHA-256("ce1/decl" ‖ CE(decl))`. Takes the CE bytes.
pub fn decl_hash(ce: &[u8]) -> Hash32 {
    tagged(b"ce1/decl", ce)
}

/// `SHA-256("ce1/name" ‖ CE(name))`.
pub fn name_hash(n: &Name) -> Hash32 {
    let mut w = Writer::new();
    w.name(n);
    tagged(b"ce1/name", &w.finish())
}

/// `SHA-256("ce1/axioms" ‖ CE(names))`, names sorted and deduplicated by the caller.
pub fn axioms_hash(names: &[Name]) -> Hash32 {
    let mut w = Writer::new();
    w.names(names);
    tagged(b"ce1/axioms", &w.finish())
}

/// SHA-256 of the concatenation of `parts` (at most 183 bytes in total), compressing
/// pre-padded blocks directly. Same result as `Sha256::digest`, without the streaming
/// buffer: Merkle node and leaf hashes are small and fixed-size, and in the zkVM the
/// buffering cost more than the (accelerated) compression.
pub fn sha256_small(parts: &[&[u8]]) -> Hash32 {
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut buf = [0u8; 192];
    let mut len = 0;
    for p in parts {
        buf[len..len + p.len()].copy_from_slice(p);
        len += p.len();
    }
    assert!(len <= 192 - 9, "sha256_small: input too long");
    buf[len] = 0x80;
    let nblocks = (len + 9).div_ceil(64);
    buf[nblocks * 64 - 8..nblocks * 64].copy_from_slice(&((len as u64) * 8).to_be_bytes());
    let mut state = H0;
    let blocks: [sha2::digest::generic_array::GenericArray<u8, sha2::digest::consts::U64>; 3] =
        core::array::from_fn(|i| *sha2::digest::generic_array::GenericArray::from_slice(&buf[i * 64..i * 64 + 64]));
    sha2::compress256(&mut state, &blocks[..nblocks]);
    let mut out = [0u8; 32];
    for (o, w) in out.chunks_exact_mut(4).zip(state) {
        o.copy_from_slice(&w.to_be_bytes());
    }
    out
}

#[cfg(test)]
mod small_tests {
    use super::*;

    #[test]
    fn sha256_small_matches_sha256() {
        for len in 0..=183usize {
            let data: alloc::vec::Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
            let (a, b) = data.split_at(len / 3);
            let want: Hash32 = Sha256::digest(&data).into();
            assert_eq!(sha256_small(&[a, b]), want, "length {len}");
        }
    }
}
