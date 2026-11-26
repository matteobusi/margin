# zkvm

RISC Zero guest (`methods/guest`) and native prover (`host`).

## Setup

Install the RISC Zero toolchain (see the RISC Zero docs for `rzup`), then:

```bash
cd zkvm
cargo run --release -p margin-host --features kernel-nanoda -- demo demo-receipt.bin   # binary: margin
cargo run --release -p margin-host --features kernel-nanoda -- image-id
```

Set `RISC0_DEV_MODE=1` for fast fake proofs while you develop.

## Kernels

The guest needs exactly one kernel feature:

| Feature | Status |
|---|---|
| `kernel-nanoda` | Works. nanoda_lib (vendored, `third_party/nanoda_lib`) via its own lean4export parser |
| `unsound-skip-typecheck` | Pipeline tests only. Never list its image ID in `checkers.json`. |

## Reproducible builds

Release image IDs must come from `cargo risczero build` (Docker), so that anyone can
rebuild the guest and get the same image ID.

## GPU proving (Apple Silicon)

`--features metal` builds the in-process prover with Metal. It needs the full Xcode
(`xcrun metal`); Command Line Tools alone are not enough.

## Real Lean input

See the README: `margin run` does export, prepare, check and prove.
