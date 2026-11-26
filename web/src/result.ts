import { h } from "./dom";
import type { VerifyResult } from "./verifier";

/** Render a verification result: verdict, authorship (if a claim was checked), report. */
export function resultView(r: VerifyResult): HTMLElement {
  const box = h("div", { class: `result ${r.valid ? "ok" : "bad"}` },
    h("p", { class: "verdict" }, r.valid ? "Valid receipt." : "Invalid receipt."),
    h("p", { class: "small" }, r.valid
      ? "A published checker accepted proofs of the theorems below, over a published version of Lean or Mathlib. The proofs are not in the receipt."
      : "Do not trust what this file claims."),
  );
  if (r.authorship) {
    box.append(h("p", { class: `author small ${r.authorship.ok ? "ok" : "bad"}` },
      h("b", {}, r.authorship.ok ? "Authorship claim valid. " : "Authorship claim rejected. "),
      r.authorship.message));
  }
  box.append(h("pre", { class: "output" }, r.report));
  return box;
}

/** A shell command with a copy link. The link copies whatever the block shows. */
export function commandBlock(text: string): HTMLElement {
  const code = h("code", {}, text);
  const btn = h("button", { class: "copy", type: "button" }, "copy");
  btn.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText((code.textContent ?? "").replace(/\\\n\s*/g, ""));
      btn.textContent = "copied";
    } catch {
      btn.textContent = "select to copy";
    }
    setTimeout(() => (btn.textContent = "copy"), 1500);
  });
  return h("div", { class: "cmd-wrap" }, h("pre", { class: "cmd" }, code), btn);
}

/** Update the text of a command block created by commandBlock. */
export function setCommand(block: HTMLElement, text: string) {
  block.querySelector("code")!.textContent = text;
}
