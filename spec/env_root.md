# Environment root, version 3

`env_root` is a Merkle root over all declarations of a trusted environment
(for example, all of `Init` for one Lean release).

## Leaves

For each declaration `d` (an inductive block is one declaration):

```
header(d)   = the Statement form of d (name, universe parameters, type) if d is a theorem,
              a safe definition or a safe opaque constant; else d itself
leaf(d)     = SHA-256(0x00 ‖ name_hash(key(d)) ‖ decl_hash(header(d)) ‖ decl_hash(d) ‖ axioms_hash(ax(d)))
```

- `decl_hash`, `name_hash`, `axioms_hash`: see `ce.md`.
- `ax(d)`: the axioms that `d` depends on, transitively, through types, values and
  proofs, including `d` itself if it is an axiom. Sorted by `Name` order, no duplicates.

Sort the leaves by `name_hash(key(d))`, ascending. Two declarations with the same key are
an error. A reference to a constant outside the environment, or a dependency cycle
between declarations, is an error.

## Tree

The RFC 9162 (Certificate Transparency v2) Merkle tree hash over the sorted leaf list,
with interior nodes:

```
node(l, r) = SHA-256(0x01 ‖ l ‖ r)
```

## Openings

The prover reveals each needed environment declaration in one of two ways:

- **Full**: its CE bytes. The verifier recomputes `header(d)` from them.
- **Statement-only** (theorems, safe definitions, safe opaques): CE bytes of `header(d)`
  and `decl_hash(d)`. The kernel sees the constant as an axiom of the same type. This only
  removes definitional equalities (the kernel can only reject more), and the committed
  `ax(d)` still stands for what `d` uses. Unsafe and partial declarations have no
  statement form: opened in full, the kernel rejects them.

Version 3 (checker v0.0.2) added safe definitions and opaques to the statement forms,
so that a prover input opens in full only the declarations the kernel unfolds (found by
a recorded native run). Version 2 roots are not compatible.

Each opening carries `ax(d)`. One multi-proof covers all openings: the leaf indices
(strictly increasing) and the roots of the maximal subtrees that contain none of them,
in left-to-right order.

## Trust

`ax(d)` is computed outside the zkVM, when the environment tree is built. It is part of
the leaf, so `env_root` commits to it, and anyone can recompute it from the official
Lean release. Trusting `env_root` therefore means trusting both the declarations and
their axiom sets.

## Published roots

```bash
margin env root init.ndjson      # output of `lean4export Init`
```

## Module interfaces

A module receipt (see `journal.md`) commits to the interface of its declarations as a
tree with exactly the same leaves, where `ax(d)` is computed by the same rule from the
committed sets of the environment and of the modules it assumed. Importers open module
declarations against `iface_root` the same way as environment declarations against
`env_root`.
