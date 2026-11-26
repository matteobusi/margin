# Canonical Encoding (CE), version 1

Status: draft. The reference implementation is `crates/canon`. If this text and the code
disagree, file a bug; do not rely on either until v1 is frozen.

## Purpose

CE gives each Lean kernel declaration exactly one byte string. The hash of a declaration
is computed over that byte string. Two parties that export the same declaration get the
same bytes and the same hash.

## Primitives

| Primitive | Encoding |
|---|---|
| `uvar` | Unsigned LEB128, minimal length (no trailing `0x80` groups) |
| `bytes` | `uvar` length, then the bytes |
| `str` | `bytes` holding UTF-8 |
| `tag` | One byte |

## Names

`Name` = `uvar` count, then each component:

- `0x00` `str` — string component
- `0x01` `uvar` — numeric component

The anonymous name has count 0.

## Levels

- `0x00` zero
- `0x01` succ `L`
- `0x02` max `L L`
- `0x03` imax `L L`
- `0x04` param `Name`

## Expressions

Lean expressions are DAGs. CE encodes each declaration with a local **expression table**.

1. Walk the expressions of the declaration in field order.
2. Use a post-order DFS (children left to right). Remove duplicates by structural equality.
3. The first occurrence gives the index. An expression refers to children by table index.

Table: `uvar` count, then the nodes:

- `0x00` bvar `uvar`
- `0x01` sort `Level`
- `0x02` const `Name` `uvar n` `Level×n`
- `0x03` app `ref f` `ref a`
- `0x04` lam `Name` `binfo` `ref ty` `ref body`
- `0x05` pi `Name` `binfo` `ref ty` `ref body`
- `0x06` let `Name` `ref ty` `ref val` `ref body` `nondep:u8`
- `0x07` natlit `bytes` (big-endian, minimal, zero = empty)
- `0x08` strlit `str`
- `0x09` proj `Name` `uvar idx` `ref e`

`ref` = `uvar` index into the table. A node can only refer to earlier nodes.

Binder names and binder info are erased: in `lam`, `pi` and `let`, the name must be the
anonymous name and `binfo` must be `0`. They do not affect type checking, Lean's own term
equality ignores them, and lean4export does not preserve them reliably (a shared subterm
can carry the binder names of another declaration). Displays invent names.
`binfo`: `0` default, `1` implicit, `2` strict implicit, `3` inst implicit.
`mdata` nodes are removed before encoding.

## Declarations

CE declarations carry exactly the fields that a Lean kernel reads, following the
lean4export format 3.1 (`crates/canon/src/lean4export.rs` converts both ways).

Each declaration: magic `"CE1"`, then the expression table, then one of:

- `0x00` axiom: `Name` `lparams` `ref ty` `unsafe:u8`
- `0x01` definition: `Name` `lparams` `ref ty` `ref val` `hints` `safety:u8`
- `0x02` theorem: `Name` `lparams` `ref ty` `ref val`
- `0x03` opaque: `Name` `lparams` `ref ty` `ref val` `unsafe:u8`
- `0x04` quot: `Name` `lparams` `ref ty` `kind:u8` (`0` type, `1` ctor, `2` lift, `3` ind)
- `0x05` inductive block:
  - `uvar n`, then per type: `Name` `lparams` `ref ty` `names all` `names ctors`
    `isRec:u8` `isReflexive:u8` `unsafe:u8` `uvar numParams` `uvar numIndices` `uvar numNested`
  - `uvar n`, then per constructor: `Name` `lparams` `ref ty` `unsafe:u8` `Name induct`
    `uvar cidx` `uvar numParams` `uvar numFields`
  - `uvar n`, then per recursor: `Name` `lparams` `ref ty` `unsafe:u8` `names all`
    `uvar numParams` `uvar numIndices` `uvar numMotives` `uvar numMinors`
    `uvar nrules`, per rule `Name ctor` `uvar nfields` `ref rhs`, then `k:u8`
- `0x06` statement: `Name` `lparams` `ref ty` (journal only; never in an environment)

`lparams` and `names` = `uvar` count, then `Name×count`.
`hints`: `0` opaque, `1` abbrev, `2` regular followed by `uvar` height.
`safety`: `0` safe, `1` unsafe, `2` partial.

The expression table follows the order of `Decl::exprs`: for an inductive block, the type
of each type, then of each constructor, then for each recursor its type and its rule
right-hand sides.

Constructors and recursors are included because a trusted environment declaration is not
re-checked, and the kernel needs them for reduction. For user declarations, the kernel
re-derives them and rejects any difference.

Fields that do not change type checking (`all` of definitions and theorems, `mdata`) are
not encoded.

## Hashes

- `decl_hash = SHA-256("ce1/decl" ‖ CE(decl))`
- `name_hash = SHA-256("ce1/name" ‖ CE(Name))`
- `axioms_hash = SHA-256("ce1/axioms" ‖ uvar n ‖ CE(Name)×n)` (sorted names)

For an inductive block, the leaf key is the name of the first type. For a quot
declaration, the key is its own name (each of the four constants is a separate leaf).
