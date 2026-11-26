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
    h("p", { class: "lead" }, "Verification runs in your browser and takes a fraction of a second. Nothing is uploaded."),
    dropZone("Drop a receipt here (and, optionally, its authorship claim .json), or click to choose", { multiple: true }, async (files) => {
      for (const f of files) {
        if (f.name.endsWith(".json")) claim = { name: f.name, text: await f.text() };
        else receipt = { name: f.name, bytes: new Uint8Array(await f.arrayBuffer()) };
      }
      run();
    }),
    chosen,
    out,
    h("h2", {}, "What a valid receipt means"),
    h("ul", {},
      h("li", {}, "A published checker (", h("a", { href: "checkers.json" }, "checkers.json"),
        ") type-checked proofs of the listed theorems inside the zkVM, over a published Lean or Mathlib version (",
        h("a", { href: "envs.json" }, "envs.json"), "). These lists never come from the prover."),
      h("li", {}, "Only the listed axioms were used. sorry, native_decide and custom axioms are always rejected."),
      h("li", {}, "The statements are shown in raw kernel form: binder names become x0, x1, … and notation is unfolded (", h("code", {}, "LE.le … a b"), " is ", h("code", {}, "a ≤ b"), ")."),
      h("li", {}, "An owner key is a pseudonym. A valid authorship claim proves that whoever signed it holds that key; " +
        "it says who they are only if they published the key somewhere tied to them."),
    ),
    h("p", {}, "To be certain the receipt proves exactly the statement you mean, write it in your own .lean file with ",
      h("code", {}, "sorry"), " and run ", h("code", {}, "margin match"), ". See ", h("a", { href: "#/about" }, "How it works"), "."),
    h("p", { class: "note" }, "No receipt at hand? Try one from the ", h("a", { href: "#/examples" }, "examples"), "."),
  );
}
