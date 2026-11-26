# Vendored risc0-zkvm-platform

- Upstream: crates.io `risc0-zkvm-platform` 2.2.3 (RISC Zero, Apache-2.0)
- Used by the Margin guest through `[patch.crates-io]` in `zkvm/methods/guest/Cargo.toml`.

## Local patch

`src/heap/embedded.rs` (feature `heap-embedded-alloc`): the TLSF heap
(`risc0-embedded-alloc`) is replaced by a size-class allocator with per-class free lists
(8-byte classes up to 512 bytes, then powers of two). In the zkVM every instruction costs
a cycle; TLSF bookkeeping was about 18% of the guest's cycles. `Cargo.toml` drops the
`risc0-embedded-alloc` dependency accordingly. Nothing else changes.
