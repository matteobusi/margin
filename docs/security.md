# Security notes

Status: pre-alpha. Nothing here has been audited.

## What a receipt reveals

A receipt is a RISC Zero **succinct** receipt (`InnerReceipt::Succinct`); the verifier
rejects every other kind (composite receipts reveal the segment count; fake dev-mode
receipts are disabled at build time with `disable-dev-mode`).

Its public claim (`ReceiptClaim`) contains:

| Field | Value in our receipts | Reveals |
|---|---|---|
| `pre` | image ID of the guest | which checker ran |
| `post` | `pc = 0`, memory root all zeros (normal halt) | nothing |
| `exit_code` | `Halted(0)` | that the check passed |
| `input` | zero digest | nothing |
| `output` | journal digest (+ empty assumptions) | the journal |

Checked on 2026-11-24 by decoding a real receipt (risc0-zkvm 3.1.0). The journal holds
the claimed statements, the definitions they depend on, `env_root`, the axioms used, the
checker name and the owner public key. It holds no proof term and no private lemma.

## Why the seal is zero-knowledge

The seal of a succinct receipt is a STARK for RISC Zero's recursion circuit, whose
witness includes the inner segment proofs. In risc0 3.x:

- `risc0-circuit-recursion/src/prove/witgen.rs` appends `ZK_CYCLES` (1024) rows of
  uniformly random field elements to the witness data columns before committing.
- `risc0-zkp/src/prove/poly_group.rs` evaluates committed polynomials on a shifted domain
  (`f(x) -> f(3x)`), so the opened points do not lie on the trace domain; with the random
  padding, the openings reveal nothing about the trace ("is zero knowledge so long as
  there is sufficient randomized padding").
- RISC Zero's README states perfect zero-knowledge with default parameters (98 bits of
  conjectured soundness).

We rely on RISC Zero's construction and analysis; we have not verified it independently.

## What you trust

1. **The checker** named by the image ID: our guest (decoder, policy, statement cone) and
   nanoda_lib. Open source, reproducible build, not formally verified.
2. **RISC Zero**: soundness of the STARK and the recursion circuit; correctness of the
   verifier library you run.
3. **`env_root`**: the environment declarations (checked by the official Lean kernel when
   Lean built them) and their transitive axiom sets (computed by `margin-envbuild`). Anyone
   can recompute it from the official release with `margin env root`.
4. **Your reading of the statement**: the checker proves the formal statement. Use
   `margin match` with your own statement file to be sure it is the statement you mean.

You do not trust the prover, their machine, the website, its server, the elaborator, or
Lake.

## Rules for verifiers

- Never accept an image ID or an `env_root` from the prover. Use the published lists, or
  rebuild them.
- The `checker` text in the journal is informational only.
- An authorship claim (`margin claim`) proves possession of the owner key. It does not prove
  identity unless the key was linked to an identity independently.

## Known limitations

- nanoda_lib is not formally verified. A soundness bug would let a false theorem through.
  A second, independent checker (lean4lean) with a two-checker policy is planned.
- Axiom sets are computed outside the zkVM (part of `env_root`).
- Inductive declarations in the environment are trusted as exported; user inductives are
  re-checked by the kernel, including their constructors and recursors.
