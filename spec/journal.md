# Journal, version 1

The journal is the public output of the guest. The receipt commits to it.

The journal does **not** contain the checker image ID. The guest cannot know its own image ID.
The verifier binds the checker: it verifies the receipt against an image ID from its list
of accepted checkers (`checkers.json`).

## Encoding

Magic `"JRN1"`, then:

| Field | Encoding | Meaning |
|---|---|---|
| `version` | `uvar` | `1` |
| `checker` | `str` | Name and version of the checker (informational only) |
| `env_root` | 32 bytes | Root of the trusted environment |
| `owner` | 32 bytes | Ed25519 public key of the prover |
| `claims` | `uvar n`, `bytes×n` | CE `statement` declarations of the claimed theorems |
| `stmt_cone` | `uvar n`, `bytes×n` | CE declarations that the statements depend on, outside the environment |
| `axioms` | `uvar n`, `Name×n` | Axioms that the claims use, sorted by name (`Name` order in `crates/canon`) |

## Statement cone rules

Start from the constants in the claimed statements. Follow references transitively.

- Stop at constants from the environment.
- Definitions, opaques and inductive blocks: include them with their values.
- Theorems: include the **type only** (as a `statement`). Do not follow their proofs.

Warning: a definition body can contain proof terms (for example, a subtype value).
Those terms become public. Keep proofs in theorems if you want to hide them.
