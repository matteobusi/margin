# Composition test

A three-module core Lean project (`CoreMods/`): `A` (a definition and a lemma proved with
`omega`), `B` (imports `A`), `Main` (imports `B`, holds the claim `main_claim`). The inputs
were prepared by `margin build` for the development guest `guest.elf`
(image `4022792e…`): `A.input` and `B.input` are module runs, `final.input` is the claim
run. `scripts/gpu-bench.sh` proves A, then B assuming A, then the final run assuming both,
and verifies the composed receipt against `checkers.json` / `envs.json`.
