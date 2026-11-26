# Deploying the website

The site is static: HTML, JS, the WASM verifier, and two trust lists
(`web/public/checkers.json`, `web/public/envs.json`). No server code, no database.

## Before the first public deploy

1. Build the guest reproducibly and publish its image ID:

   ```bash
   cd zkvm
   MARGIN_REPRODUCIBLE=1 cargo run --release -p margin-host --features kernel-nanoda -- image-id
   ```

   Put the ID in `web/public/checkers.json` with status `experimental`. Anyone can
   rebuild it with the same command and compare.
2. Rebuild `env_root` for each supported Lean release and check it matches
   `web/public/envs.json`.

## Release (binaries)

1. Freeze the guest code. Build reproducibly and put the image ID in
   `web/public/checkers.json` (step 1 above). A second clean build must print the same ID.
2. Tag `vX.Y.Z` and push the tag: `.github/workflows/release.yml` builds `margin` for Linux
   (guest built in Docker) and macOS (embedding the same guest binary), checks the image ID
   against `checkers.json`, and publishes binaries with checksums.
3. Upload the environment caches (for example `mathlib.cache`, 1.8 GB) to the release by
   hand, and list their roots in the release notes.

## Deploy

The site is served by GitHub Pages at <https://matteobusi.github.io/margin/>, built by
`.github/workflows/deploy.yml` on every push to `main`. One-time setup: in the repository,
Settings → Pages → Source: "GitHub Actions". All URLs in the site are relative, so it works
under any sub-path or a custom domain (add `web/public/CNAME` for the latter).

`web/public/_headers` (cross-origin isolation) is ignored by GitHub Pages. The verifier does
not need it; a future multi-threaded in-browser prover would.

Local preview:

```bash
cd web && npm run wasm && npm run build && npx vite preview
```
