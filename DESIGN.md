# Design

> Working name: **margin** (placeholder). Run `scripts/rename.sh` to change it.

## Goal

A user drops a Lean 4 project in the browser. The browser proves, in zero knowledge,
that a Lean kernel type checker accepts the project. The output is a receipt.
Any person can verify the receipt: in the browser, with a CLI, or with a library.

- The **statements** of the claimed theorems are public.
- The **proofs** stay secret. They never leave the user's machine.

## Decisions

| Topic | Decision |
|---|---|
| zkVM | RISC Zero (receipts are zero-knowledge; SP1 STARKs are not) |
| Proof system | STARK succinct receipt only. No trusted setup. |
| Groth16 wrap | Future milestone (compact certificate, server-side wrap) |
| Checker | `nanoda`-based Rust kernel first. `lean4lean` later. |
| Proving location | Browser first. Optional desktop helper later. |
| Statement secrecy | Not in v1. Salted-hash mode is a future flag. |
| Environment | One environment per Mathlib commit (includes Lean core). Cached in binary form. |
| Prover identity | Pseudonymous Ed25519 owner key in the journal; signed authorship claims |

## Components

| Path | Component | Runs in |
|---|---|---|
| `web/` | Web app (Vite, TypeScript) | Browser |
| `crates/canon` | Canonical encoding (CE), hashes, Merkle trees | WASM, RISC-V, native |
| `crates/journal` | Public journal format | WASM, RISC-V, native |
| `crates/checker` | Axiom policy, statement cone, kernel interface | WASM, RISC-V, native |
| `crates/verify` | Receipt verifier, library + CLI | WASM, native |
| `crates/envbuild` | Environments (export or cache), prepare, pre-check, statement matching | Native |
| `crates/kernel-nanoda` | Kernel adapter for the vendored nanoda_lib | WASM, RISC-V, native |
| `zkvm/methods/guest` | RISC Zero guest program | zkVM |
| `zkvm/host` | The `margin` CLI: keygen, export, run, prove, verify, match, claim | Native |
| `server/` | CDN layout and registry (later) | Server |

## Data formats

See `spec/` for the normative text.

- **CE**: a versioned binary encoding of kernel declarations. One declaration has exactly one encoding.
  The hash of a declaration is `SHA-256(CE(decl))`.
- **env_root**: the Merkle root over all declarations of a trusted environment
  (one Lean toolchain; later one Mathlib commit). Anyone can rebuild it with `envbuild`.
- **Journal** (public output of the guest):

```
version, checker (informational), env_root,
claims:     [statement]        -- claimed theorems (name + type)
stmt_cone:  [decl]             -- user definitions that the statements use
axioms:     [name]
owner:      ed25519 public key
```

The journal cannot contain the checker image ID (the guest does not know its own ID).
The verifier binds the checker: it verifies the receipt against each image ID in its
accepted list (`checkers.json`).

## Guest logic

Inputs:

- Private: user declarations; environment declarations that they use, with Merkle paths.
- Public: `env_root`, `owner`, names of the claimed theorems.

Steps:

1. Verify one Merkle multi-proof for all environment openings against `env_root`.
   Library theorems are usually opened statement-only (no proof).
2. Add the environment declarations without a check. `env_root` makes them trusted.
3. Type-check each user declaration in dependency order.
4. Reject `sorryAx`, `Lean.ofReduceBool`, `Lean.trustCompiler` and all axioms that the user declares.
5. Record the axioms that the claimed theorems use: walk user declarations, and use the
   committed axiom set of each environment declaration they reach. They must be a
   subset of `{propext, Quot.sound, Classical.choice}`.
6. Calculate the statement cone of the claimed theorems.
7. Write the journal. Never write proof terms or private lemmas.

## User flow (prover)

1. Drop a project folder or zip.
2. Read `lean-toolchain`. Reject unsupported toolchains.
3. Lean worker (WASM) loads the needed `.olean` files from the CDN, caches them in OPFS, elaborates, exports.
4. `canon` encodes the declarations and gets Merkle paths for the dependencies.
5. The user selects the theorems to claim.
6. Pre-check with the checker in WASM. Show errors, axioms, cycle estimate.
7. Prove segments in Web Workers. Keep segment receipts in OPFS (resumable). Join into one succinct receipt.
8. Download `receipt.bin`. Optional: register `(journal hash, time)`.

## Verifier flow

1. Open the verify page, or run `margin-verify receipt receipt.bin`.
2. Check the receipt against the accepted image IDs (`checkers.json`) and the journal's
   `env_root` against the accepted environments (`envs.json`). Never take either from the
   prover: a modified checker has a different image ID, and a fake library has a
   different root. The verifier is built with RISC Zero's `disable-dev-mode`.
3. Show the journal.
4. To be sure the receipt proves *your* statement: write it in your own `.lean` file
   (proof `sorry`), export it, and run `margin-envbuild match`. It recomputes the claims and
   the statement cone with the same code as the guest and compares bytes. MATCH means
   the same statement, the same definitions, the same library.

## Owner keys and authorship

The journal's `owner` is an Ed25519 public key chosen by the prover (`margin keygen`). It is
part of the proven statement. `margin claim` signs
`"ce1/authorship/v1" ‖ SHA-256(journal) ‖ len(note) ‖ note` with the secret key; anyone can
check the claim against the receipt (`margin verify --claim`). Copying a receipt does not let
someone claim it. The key is pseudonymous unless its holder links it to an identity.

## Trust model

Trusted: the checker that the image ID names (reproducible build), RISC Zero STARK
soundness, and `env_root`: the environment declarations (the official Lean kernel checked
them) and their transitive axiom sets (computed by `margin-envbuild`). Anyone can
recompute `env_root` from the official Lean release.

Not trusted: the browser, the elaborator, Lake, the CDN, the server.

Known gap: no checker is fully verified yet. Label the project **alpha**.

## Prior work

- zkPi (Laufer, Ozdemir, Boneh, CCS 2024): a custom zkSNARK for Lean proofs. Partial coverage.
  This project runs a complete kernel in a general zkVM instead.

## Milestones

| M | Scope |
|---|---|
| M0 | Spike: kernel cycles in RISC Zero, browser prover speed, zk of succinct receipt, Lean WASM loading |
| M1 | Native CLI end to end, core Lean only |
| M2 | Verifier in browser and CLI |
| M3 | Proving in browser |
| M4 | Lean elaboration in browser |
| M5 | Mathlib environments |
| M6 | Second checker (`lean4lean`), two-checker policy |
| M7 | Registry and challenge pages |
| M8 | Compact certificate (Groth16 wrap) |
