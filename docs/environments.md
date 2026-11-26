# Environments

An environment is the trusted library your proof builds on, committed to by `env_root`
(see `spec/env_root.md`). Accepted environments are listed in `web/public/envs.json`.

| Environment | env_root | Declarations |
|---|---|---|
| Mathlib 5d3d31c (Lean v4.37.0-rc1) | `fb4484bb…7a67` | 733,551 |
| Lean v4.36.0 `Init` | `275127d9…bb19` | 58,547 |

## Getting the Mathlib environment

`margin prepare` / `margin run` need the environment as a cache file (1.8 GB for Mathlib).
Either download it from the release assets (then check its root with
`margin env root mathlib.cache`), or build it yourself:

```bash
git clone https://github.com/leanprover-community/mathlib4 && cd mathlib4
git checkout 5d3d31c
lake exe cache get
# lean4export (df81913) built with Mathlib's toolchain (leanprover/lean4:v4.37.0-rc1)
lake env lean4export Mathlib > mathlib.ndjson      # ~28 min, 13 GB RAM, 6.4 GB
margin env cache mathlib.ndjson mathlib.cache         # ~4 min, 9.5 GB RAM
margin env root mathlib.cache                         # must print fb4484bb…7a67
```

## Your project

Your Lake project must use the same Mathlib commit (and so the same Lean toolchain). Any
library declaration in your export that differs from the environment is reported by
`margin prepare` ("different Lean or Mathlib version?").

## Cost

A theorem only pulls in what it uses. `infinitely_many_primes` needs 219 of Mathlib's
733,551 declarations (200 full, 19 statement-only): a 137 KB prover input and 30.7M guest
cycles, about an hour of CPU proving on 16 cores. Larger tactic proofs (simp, norm_num,
omega) produce larger terms and cost more; run `margin execute` first to see the cycle count.
