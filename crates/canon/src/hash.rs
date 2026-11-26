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
