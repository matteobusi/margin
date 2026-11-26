import { h } from "./dom";
import type { VerifyResult } from "./verifier";

/** Render a verification result: verdict, authorship (if a claim was checked), report. */
export function resultView(r: VerifyResult): HTMLElement {
  const box = h("div", { class: `result ${r.valid ? "ok" : "bad"}` },
    h("p", { class: "verdict" }, r.valid ? "✓ Valid receipt" : "✗ Invalid receipt"),
    h("p", { class: "note" }, r.valid
      ? "A published checker accepted proofs of the statements below, over a published Lean environment. The proofs are not in the receipt."
      : "Do not trust what this file claims."),
  );
  if (r.authorship) {
    box.append(h("p", { class: `author ${r.authorship.ok ? "ok" : "bad"}` },
      h("strong", {}, r.authorship.ok ? "✓ Authorship claim valid. " : "✗ Authorship claim rejected. "),
      r.authorship.message));
  }
  box.append(h("pre", { class: "output" }, r.report));
  return box;
}

/** A shell command with a copy button. The button copies whatever the block shows. */
export function commandBlock(text: string): HTMLElement {
  const code = h("code", {}, text);
  const btn = h("button", { class: "copy", type: "button", "aria-label": "Copy command" }, "Copy");
  btn.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(code.textContent ?? "");
      btn.textContent = "Copied";
    } catch {
      btn.textContent = "Select and copy";
    }
    setTimeout(() => (btn.textContent = "Copy"), 1500);
  });
  return h("div", { class: "cmd-wrap" }, h("pre", { class: "cmd" }, code), btn);
}

/** Update the text of a command block created by commandBlock. */
export function setCommand(block: HTMLElement, text: string) {
  block.querySelector("code")!.textContent = text;
}
