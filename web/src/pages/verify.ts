import { h, dropZone } from "../dom";
import { resultView } from "../result";
import { verifyPublished } from "../verifier";

export function renderVerify(root: HTMLElement) {
  let receipt: { name: string; bytes: Uint8Array } | null = null;
  let claim: { name: string; text: string } | null = null;
  const chosen = h("p", { class: "note" });
  const out = h("div", { "aria-live": "polite" });

  const run = async () => {
    chosen.textContent = [receipt && `Receipt: ${receipt.name}`, claim && `Authorship claim: ${claim.name}`]
      .filter(Boolean).join(" · ");
    if (!receipt) {
      out.replaceChildren(h("p", { class: "note" }, "Now add the receipt."));
      return;
    }
    out.replaceChildren(h("p", { class: "note" }, "Checking…"));
    try {
      out.replaceChildren(resultView(await verifyPublished(receipt.bytes, claim?.text)));
    } catch (e) {
      out.replaceChildren(h("p", { class: "status bad" }, String(e instanceof Error ? e.message : e)));
    }
  };

  root.append(
    h("h1", {}, "Verify a receipt"),
    h("p", { class: "subtitle" }, "In this browser. Nothing is uploaded."),
    dropZone("Drop a receipt here, with its authorship claim (.json) if it has one, or click to choose files.", { multiple: true }, async (files) => {
      for (const f of files) {
        if (f.name.endsWith(".json")) claim = { name: f.name, text: await f.text() };
        else receipt = { name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) };
      }
      run();
    }),
    chosen,
    out,
    h("p", { class: "note" }, "No receipt at hand? Try one of the ", h("a", { href: "#/examples" }, "examples"), "."),

    h("h2", {}, "What a valid receipt tells you"),
    h("p", {},
      "That a published checker (", h("a", { href: "checkers.json" }, "checkers.json"), ") type-checked proofs " +
      "of the listed theorems over a published version of Lean or Mathlib (", h("a", { href: "envs.json" }, "envs.json"),
      "), using only the listed axioms. Both lists come with this site and its source code, never from the prover."),
    h("p", {},
      "The statements are printed as the kernel sees them: bound variables are named x0, x1, … and notation is " +
      "unfolded, so ", h("code", {}, "LE.le … a b"), " means ", h("code", {}, "a ≤ b"), ". To be sure that the " +
      "receipt proves exactly the statement you have in mind, write it in your own .lean file with ",
      h("code", {}, "sorry"), " and compare with ", h("code", {}, "margin match"), "."),
    h("p", {},
      "An owner key is a pseudonym. A valid authorship claim shows that whoever signed it holds that key; it says " +
      "who they are only if they have published the key somewhere tied to their name."),
  );
}
