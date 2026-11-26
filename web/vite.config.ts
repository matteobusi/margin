import { defineConfig } from "vite";

// Cross-origin isolation: needed for SharedArrayBuffer (multi-threaded WASM prover).
// Production hosts must send the same headers (see public/_headers).
const isolation = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

export default defineConfig({
  server: { headers: isolation },
  preview: { headers: isolation },
  worker: { format: "es" },
  build: { target: "es2022" },
  // Relative asset URLs: the site is served from a sub-path (https://matteobusi.github.io/margin/).
  base: "./",
});
