// Loads the verifier WASM (built from crates/verify-wasm with `npm run wasm` into
// public/wasm/verify, served as static files).

export interface VerifyResult {
  valid: boolean;
  report: string;
  /** Only set when an authorship claim was given and the receipt is valid. */
  authorship: null | { ok: boolean; message: string };
}

export interface Verifier {
  verifyReceipt(receipt: Uint8Array, checkersJson: string, envsJson: string, claimJson?: string): VerifyResult;
  inspectJournal(journal: Uint8Array): string;
}

let cached: Promise<Verifier> | null = null;

export function loadVerifier(): Promise<Verifier> {
  cached ??= (async () => {
    // The module does not exist until `npm run wasm` runs. Keep the import dynamic
    // and opaque to the bundler so the app builds without it.
    // Resolved against the page, so the site works under any base path (e.g. /margin/).
    const path = new URL("wasm/verify/margin_verify_wasm.js", document.baseURI).href;
    let mod: any;
    try {
      mod = await import(/* @vite-ignore */ path);
    } catch {
      cached = null;
      throw new Error("The verifier could not be loaded. Reload the page and try again.");
    }
    await mod.default();
    return {
      verifyReceipt: (r, c, e, claim) => JSON.parse(mod.verify_receipt(r, c, e, claim)) as VerifyResult,
      inspectJournal: (j) => mod.inspect_journal(j) as string,
    };
  })();
  return cached;
}

/** Verify against the trust lists published with this site (checkers.json, envs.json).
 *  They are also in the public repository, so anyone can compare them with reproducible builds. */
export async function verifyPublished(receipt: Uint8Array, claimJson?: string): Promise<VerifyResult> {
  const [v, checkers, envs] = await Promise.all([
    loadVerifier(),
    fetch("checkers.json", { cache: "no-store" }).then((r) => r.text()),
    fetch("envs.json", { cache: "no-store" }).then((r) => r.text()),
  ]);
  return v.verifyReceipt(receipt, checkers, envs, claimJson);
}
