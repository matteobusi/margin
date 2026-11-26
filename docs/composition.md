# Large developments: cheaper guest, per-module receipts, GPU clusters

Status: implemented (steps 1-4), 2026-11-26. Target: grh-compact (Lean v4.35.0, Mathlib 56a0058).

## Validation so far

Composition, end to end, on an RTX 5090 (`bench/compose`, three modules, core Lean):

| Run | Cycles (padded) | 5090 |
|---|---|---|
| module `A` (definition, `omega` lemma) | 56.6 M | 67.0 s |
| module `B`, assuming `A` | 2.6 M | 3.7 s |
| final run, assuming `A` and `B` | 2.1 M | 3.0 s |

The composed receipt (224 KB) verifies; it shows only the claim and the definition its
statement uses, and the axioms of module `A` reach the claim. Proving the final run
without one of its module receipts fails. Unit tests cover: another image or environment,
an unverified module receipt, a tampered module opening, a hidden definition in the
statement, `sorry` inside a module.

Cheaper guest (step 1) on sample A: 2.55 G → 0.78 G padded cycles.

Still to measure: the cycle count of all of grh-compact (`margin build --check-only
--cycles`), and a real Bento cluster run.

## Measurements that drive the design

grh-compact, five main theorems, checked natively with the release checker:

| | |
|---|---|
| User declarations | 37,532 (plus 16,241 Mathlib declarations opened in full, 40,759 as statements) |
| Prover input | 672 MB |
| Native check (nanoda, one thread) | 1,090 s, 8 GB, ACCEPTED, axioms `propext`, `Quot.sound`, `Classical.choice` |

One receipt for everything is impossible: the guest has about 3 GB of memory.

Calibration in the zkVM (executor, no proof):

| Sample | User decls | Env decls | Native | zkVM cycles (real / padded) |
|---|---|---|---|---|
| A `exists_reduced_closure` | 7 | 5,896 | 0.2 s | 1.28 G / 2.55 G |
| B `BlochKato` (2 theorems) | 1,582 | 15,209 | 9.5 s | out of memory (bump allocator) |

Where the cycles of sample A go:

| Phase | Cycles | Share |
|---|---|---|
| Decode the env declarations into the arena | 577 M | 45 % |
| Merkle inclusion of the env openings | 91 M | 7 % |
| Convert arena to nanoda records, nanoda hash-consing | 333 M | 26 % |
| nanoda type checking | 230 M | 18 % |
| Rest (input, user decls, policy, journal) | 54 M | 4 % |

The padded count is twice the real one: paging (memory touched per segment) doubles it.

Conclusions: (1) most of the cost is moving data, not checking; (2) the guest must free
memory; (3) a module must open only the library declarations it really needs.

## Phase 1: cheaper guest (no change to what is checked)

1. **Allocator.** Use a reclaiming allocator in the guest (RISC Zero's
   `heap-embedded-alloc`, or an arena per phase that is dropped). Removes the out-of-memory
   failures; measure the cycle cost.
2. **One conversion instead of two.** Today: CE bytes → our arena → nanoda records →
   nanoda's own tables. Build nanoda's tables directly from CE bytes, so every term is
   hashed and interned once.
3. **Fewer page-ins.** Smaller, contiguous data structures (u32 ids, no `Box` per node,
   no per-declaration `Vec`s), so that each segment touches fewer pages.
4. **Profile-guided openings.** The native pre-check runs exactly the same kernel, so it
   can record which environment constants the kernel actually unfolds. `prepare` opens
   those in full and the rest as statements only. Soundness does not depend on this list:
   turning a definition into an opaque constant of the same type only removes definitional
   equalities, so it can make the checker reject, never accept something new. If the guest
   needs a value it was not given, it fails and the prover re-prepares with more openings.
   This is optional (see "Decisions").

Goal: sample A from 1.28 G to well under 0.5 G real cycles, B within memory.

## Phase 2: per-module receipts (composition)

A **module receipt** says: "these declarations type-check, assuming the interfaces of
these imported modules, over environment `env_root`". It is an ordinary succinct receipt
of the same guest, run in module mode. Its journal (`MOD1`):

| Field | Meaning |
|---|---|
| `self_id` | image ID that imported module receipts must have (input, committed) |
| `env_root` | as today |
| `module` | Lean module name (informational) |
| `iface_root` | Merkle root of the module's interface (below) |
| `deps` | list of `(module, iface_root)` of the imports it used |

**Interface.** The module's declarations, as a Merkle tree with exactly the environment's
leaf format: `leaf = H(0x00 ‖ name_hash ‖ header_hash ‖ full_hash ‖ axioms_hash)`. An
importer opens definitions in full and theorems as statements, against `iface_root`, just
like it opens Mathlib against `env_root`. Proofs never leave the module that contains them.
Axiom sets are committed per declaration, so axioms propagate across modules.

**Guest, module mode.** Input: own declarations, env openings, openings of imported
interfaces, and for each import its `MOD1` journal. For each import the guest calls
`env::verify(self_id, journal)`, then requires `journal.self_id == self_id` and
`journal.env_root == env_root`, and checks the openings against `journal.iface_root`.
Then it checks its own declarations exactly as today and commits `MOD1`.

**Final step.** The top run (for example `Main`) uses claim mode: it opens the claimed
theorems and the definitions their statements use from the interfaces, and commits the
public journal as today plus `assumed_id` (`JRN2`). The host proves it with every module
receipt added as an assumption (`ExecutorEnv::add_assumption`) and asks for a succinct
receipt: RISC Zero then folds all assumptions into one receipt with no assumptions left.

**Verifier.** Unchanged except one rule: `journal.assumed_id` must equal the receipt's
image ID (or be zero when there were no imports). Module receipts, their journals and
interfaces never appear in the final receipt; the website verifies it in a fraction of a
second, as today.

**Soundness sketch.** By induction over the import graph: each module receipt was produced
by the guest with image `self_id`, which only commits `MOD1` after nanoda accepted the
module's declarations against an environment made of `env_root` openings and imported
interfaces, each of which is in turn backed by a verified module receipt of the same
image. The final receipt's image is checked by the verifier against the published list,
and `assumed_id` ties every inner receipt to that same image.

**Large modules.** A file that is too big for one run is split into chunks (declarations in
dependency order); a chunk is a pseudo-module with the same journal.

## Phase 3: `margin build` (incremental, parallel)

1. Ask Lean (a small script run with `lake env lean`) for the module of every declaration
   and the import graph; export with lean4export once; split the export by module.
2. Cache key of a module receipt:
   `H(image_id ‖ env_root ‖ module CE bytes ‖ iface_root of each import)`.
   Receipts live in `~/.margin/cache/`. A change re-proves only the changed modules and,
   when an interface changes, their importers.
3. Schedule in dependency order; every module whose imports are done can start. Run jobs
   locally or on a cluster (`--cluster URL`).
4. Finish with the claim run.

## Phase 4: GPU clusters

RISC Zero's open-source **Bento** stack runs a proving cluster: a coordinator splits each
job into segments, GPU agents prove them in parallel, recursion joins them. `margin prove`
and `margin build` gain `--cluster URL` and submit jobs (image, input, assumption receipts)
to it. Two levels of parallelism: modules across jobs, segments within a job.

Privacy: cluster machines see the inputs. Fine for public developments (grh-lean); for
secret proofs only on machines you trust. `--cluster` says so.

Hardware: NVIDIA only (CUDA). RTX 4090 / 5090 for cost per cycle; benchmark first with
`scripts/gpu-bench.sh`.

## Cost model

`total ≈ Σ_modules (plumbing(module) + check(module)) + final run`. Phase 1 shrinks
plumbing; profile-guided openings shrink the env part of plumbing, which today dominates.
The figure for grh-compact will come from Phase 1 measurements on the calibration samples
plus a GPU benchmark (cycles per second per card).

## Decisions for the maintainer

- Profile-guided openings (Phase 1.4) do not reduce what is checked, only which library
  values the guest receives. Off by default until reviewed?
- Granularity: Lean files (792 for grh-compact) or larger groups (directories)?

## Plan

| Step | Effort |
|---|---|
| 1. Cheaper guest (allocator, direct nanoda build, layout), re-measure A, B | 3-5 days |
| 2. Module mode + claim mode, `MOD1`/`JRN2`, verifier rule, tests | 4-6 days |
| 3. `margin build`: split export, cache, scheduler | 3-4 days |
| 4. `--cluster` with Bento | 2-3 days |
| 5. grh-compact end to end on rented GPUs | depends on the benchmark |
