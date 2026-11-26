# M0 spike: measurements

Machine: Apple M2 (8 cores, 16 GB). RISC Zero 3.1 (`r0vm` from rzup, CPU proving).
Checker: nanoda_lib 0.4.32 (vendored), Lean 4.36.0, lean4export 3.1.

## Pipeline

```bash
lean -o Demo.olean Demo.lean
LEAN_PATH="$(lean --print-libdir):." lean4export Demo -- <decls> > demo.ndjson
LEAN_PATH="$(lean --print-libdir)" lean4export Init > init.ndjson     # 348 MB, 58,547 decls
margin-envbuild prepare init.ndjson demo.ndjson input.bin double_comm   # cone: 277 env decls
margin-envbuild check input.bin                                         # native: 18 ms
margin prove input.bin receipt.bin                                      # zkVM
```

## Results

| Input | Guest cycles | Real proof (CPU) | Peak RAM |
|---|---|---|---|
| Test kernel, toy env | 262K | 47 s | 2.0 GB |
| nanoda, 2 inductives + 2 theorems | 1.1M | 197 s | 2.9 GB |
| nanoda, real `Demo.lean` (`Init` cone of 277 decls), first version | 292M | not run | — |
| same, after round 1 (I/O, SHA accel, arena, typed records) | 45.6M | not run | — |
| same, after round 2 (multi-proof, statement-only library theorems) | **20.4M** | **42 min** (Linux VM, 16 cores, shared) | 9.6 GB |

Throughput: about 5,000 cycles/s on CPU (r0vm 3.1). Receipts: 223 KB (succinct).

## Linux VM (Phase 0)

16 cores, 31 GB RAM, AMD Instinct MI210 GPU (not usable: RISC Zero accelerates only CUDA
and Metal). CPU proving: 20.4M cycles in 42 min (≈ 8,100 cycles/s, with other builds
running), against ≈ 5,000 cycles/s on the M2.

Mathlib (commit 5d3d31c, Lean v4.37.0-rc1): `lean4export Mathlib` takes 28 min and 13 GB
RAM, and produces 6.4 GB / 114M lines.

Release receipts (image `a678bb86…`, before the rename to Margin; idle machine, CPU only):

| Example | Cycles | Prove | Peak RAM | Receipt | Verify (native / WASM) |
|---|---|---|---|---|---|
| `infinitely_many_primes` (Mathlib, 219 env decls) | 30.5M | 61 min | 9.6 GB | 219 KB | 0.09 s / 37 ms |
| `double_comm` (core Lean) | 20.4M | 42 min | 9.6 GB | 219 KB | 0.09 s / 23 ms |

`margin match` against Mathlib: 128 s, 7.5 GB (almost all of it loading the 1.8 GB cache).

## Where the cycles go (real `Demo.lean`)

| Phase | First version | Round 1 | Round 2 |
|---|---|---|---|
| Read input | 35M | 0.02M | 0.01M |
| Decode input frame | 1.3M | 1.3M | 0.5M |
| Decode env declarations | 58M | 13.5M | 6.6M |
| Merkle inclusion | 53M | 8M | 2.8M |
| User declarations + policy | 1M | 0.3M | 0.3M |
| Kernel input (export → nanoda) | 101M | 11.3M | 5.5M |
| **nanoda type check** | **0.6M** | **0.6M** | **0.6M** |
| Statement cone, journal | 1.6M | 1.4M | 0.4M |
| Paging / padding | ~41M | ~9M | ~3.6M |
| **Total** | **292M** | **45.6M** | **20.4M** |
| Prover input size | 236 KB | 236 KB | 84 KB |
| Environment declarations opened | 277 | 277 | 159 full + 10 statement-only |

Round 2:
- One Merkle multi-proof instead of 277 separate paths (input 236 → 153 KB).
- Environment leaves commit to header, full hash and transitive axiom set, so library
  theorems are opened statement-only and their proofs stay out of the zkVM
  (`--open-theorems` is the fallback if the kernel ever needs to unfold one).

## Conclusions

1. Type checking is about 1% of the cost. Data movement dominates.
2. Remaining ideas, by expected gain:
   - Decode: intern names and levels (fewer string allocations and comparisons).
   - Records: emit nanoda's DAG entries without the intermediate record structs.
   - Open definitions lazily (only those the kernel actually unfolds), found by a native
     dry run.
3. GPU proving (Metal) needs full Xcode (the `metal` compiler). Build with
   `--features metal` on the host.
4. In-browser proving needs roughly another order of magnitude fewer cycles, or GPU
   proving in the browser.
