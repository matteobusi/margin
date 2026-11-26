# Journal, version 2

The journal is the public output of the guest. The receipt commits to it.

The journal does **not** contain the checker image ID. The guest cannot know its own image ID.
The verifier binds the checker: it verifies the receipt against an image ID from its list
of accepted checkers (`checkers.json`).

## Encoding

Magic `"JRN2"` (version 1: `"JRN1"`, no `assumed_id`), then:

| Field | Encoding | Meaning |
|---|---|---|
| `version` | `uvar` | `2` |
| `checker` | `str` | Name and version of the checker (informational only) |
| `env_root` | 32 bytes | Root of the trusted environment |
| `owner` | 32 bytes | Ed25519 public key of the prover |
| `claims` | `uvar n`, `bytes×n` | CE `statement` declarations of the claimed theorems |
| `stmt_cone` | `uvar n`, `bytes×n` | CE declarations that the statements depend on, outside the environment |
| `axioms` | `uvar n`, `Name×n` | Axioms that the claims use, sorted by name (`Name` order in `crates/canon`) |
| `assumed_id` | 32 bytes | Image ID of the module receipts this run assumed; zeros if none |

**Verifier rule.** If `assumed_id` is not all zeros, it must equal the image ID that the
receipt verifies against. Otherwise reject: the assumed module receipts could come from
an unaccepted program.

## Statement cone rules

Start from the constants in the claimed statements. Follow references transitively.

- Stop at constants from the environment.
- Definitions, opaques and inductive blocks: include them with their values.
- Theorems: include the **type only** (as a `statement`). Do not follow their proofs.

Warning: a definition body can contain proof terms (for example, a subtype value).
Those terms become public. Keep proofs in theorems if you want to hide them.

A module declaration in the cone must have been opened in full; a statement-only opening
there is rejected (it could hide what a definition means).

## Module journal (`MOD1`)

The output of a module run: "the declarations whose interface is `iface_root` type-check
over `env_root`, assuming module receipts of image `self_id`". Module receipts are
intermediate. They are assumed by other runs of the same image (RISC Zero composition,
`env::verify`) and resolved into the final receipt; a verifier never accepts one as a
final receipt (the magic differs).

| Field | Encoding | Meaning |
|---|---|---|
| `self_id` | 32 bytes | Image ID required of assumed module receipts (and of this one's assumers) |
| `env_root` | 32 bytes | Root of the trusted environment |
| `module` | `Name` | Lean module name (informational) |
| `iface_root` | 32 bytes | Root of the module's interface tree (`env_root.md`, "Module interfaces") |
| `iface_size` | `uvar` | Number of interface leaves |
| `deps` | `uvar n`, 32 bytes×n | Interface roots of the assumed modules (informational) |

A run that assumes a module receipt requires its `self_id` and `env_root` to equal its own,
checks the receipt with `env::verify(self_id, journal)`, and opens the module's
declarations against `iface_root` with one multi-proof per module.

**Soundness sketch.** By induction on the module graph: a module journal exists only if
the image `self_id` accepted the module's declarations against the environment and
interfaces of earlier module receipts of the same image. The final receipt's image is in
the published list, and the verifier rule above ties `assumed_id`, hence every `self_id`
in the graph, to that image.
