//! Canonical encoding (CE) of Lean 4 kernel declarations.
//!
//! See `spec/ce.md`. The encoding is deterministic: [`decode_decl`] rejects any byte
//! string that [`encode_decl`] would not produce.

#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

mod encode;
pub mod fxhash;
mod hash;
pub mod lean4export;
pub mod merkle;
mod types;
pub mod wire;

pub use encode::{decode_decl, decode_decl_trusted, encode_decl};
pub use hash::{axioms_hash, decl_hash, name_hash, Hash32};
pub use types::*;
pub use wire::Error;
