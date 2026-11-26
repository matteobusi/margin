# Margin

> *The proof fits in the margin, and stays there.*
>
> Status: **pre-alpha**. The checker is not formally verified.

Prove that a Lean 4 kernel accepts your proof, without showing the proof.

You run one script in your Lean project. It type-checks your theorems inside the RISC Zero
zkVM, on your machine, and produces a zero-knowledge receipt. The receipt shows the theorem
statements. It hides the proofs. Anyone can verify the receipt, in a browser or with the CLI.

See [DESIGN.md](DESIGN.md) for the architecture and [spec/](spec/) for the formats.

## Layout

```
crates/canon      canonical encoding, hashes, Merkle trees
crates/journal    public journal format
crates/checker    axiom policy, statement cone, kernel interface
crates/verify     receipt verifier (library + CLI)
crates/verify-wasm browser bindings for the verifier
crates/envbuild   builds env_root from an exported environment
zkvm/             RISC Zero guest + native host (separate workspace)
web/              browser app (Vite + TypeScript)
spec/             normative formats
scripts/          helper scripts (rename, etc.)
```

## Use

The easy way: in your Lake project, run the script served by the website
([`web/public/prove.sh`](web/public/prove.sh)). It downloads `margin`, builds lean4export,
gets the environment cache, creates an owner key in `~/.margin`, pre-checks, and proves:

```bash
bash prove.sh [--check-only] [--anonymous] MyProject.Basic my_theorem
```

By hand, with the CLI:

```bash
margin keygen owner.key                          # pseudonymous prover key (once)
margin env cache mathlib.ndjson mathlib.cache    # environment cache (once per Mathlib version)
margin run --key owner.key mathlib.cache MyProject.MyFile receipt.bin my_theorem
margin verify receipt.bin                        # anyone: checks image ID and env_root
margin match receipt.bin mathlib.cache statement.ndjson my_theorem   # proves *your* statement?
margin claim --note "…" receipt.bin owner.key > claim.json           # authorship
margin verify --claim claim.json receipt.bin
```

Large developments (thousands of declarations): prove module by module. Every module gets
its own receipt, cached by content; the final receipt composes them and looks exactly
like a single-run receipt to verifiers.

```bash
margin build --check-only --cycles <env.cache> Main main.receipt thm1 thm2   # check everything, exact cost
margin build <env.cache> Main main.receipt thm1 thm2                          # prove (local CPU)
margin build --cluster http://localhost:8081 <env.cache> Main main.receipt thm1 thm2   # prove on GPUs
```

See [docs/composition.md](docs/composition.md) and [docs/cluster.md](docs/cluster.md).

`margin run` = `margin export` (lean4export) + `margin prepare` + `margin check` (native, milliseconds)
+ `margin prove` (zkVM). `lean4export` must be built for your Lean version and found via
`$MARGIN_LEAN4EXPORT` or `PATH`.

## Build

```bash
cargo test --workspace                       # core crates
cd zkvm && cargo build --release -p margin-host --features kernel-nanoda   # the margin CLI (needs rzup)
cd web && npm ci && npm run wasm && npm run dev                         # web app
```

Release builds: see [docs/DEPLOY.md](docs/DEPLOY.md) and `.github/workflows/release.yml`.
Security model: [docs/security.md](docs/security.md). Environments (Mathlib): [docs/environments.md](docs/environments.md). Measurements: [docs/m0-spike.md](docs/m0-spike.md).

Rename the project:

```bash
scripts/rename.sh <slug> "Display Name"
```

## License

Apache-2.0 OR MIT.
