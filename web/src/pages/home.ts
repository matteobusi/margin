import { BRAND } from "../brand";
import { h } from "../dom";

export function renderHome(root: HTMLElement) {
  root.append(
    h("h1", {}, BRAND.name),
    h("p", { class: "subtitle" }, BRAND.tagline),

    h("p", {},
      "Margin produces a small certificate, a ", h("em", {}, "receipt"), ", stating that a Lean 4 proof passes " +
      "the kernel's type check. The receipt names the theorems and the definitions their statements use. " +
      "It does not contain the proof. Anyone can check a receipt in a browser in a fraction of a second, " +
      "without installing Lean or Mathlib."),
    h("p", {}, "This is useful in two situations:"),
    h("ul", {},
      h("li", {}, h("strong", {}, "Large developments. "),
        "A reviewer of a paper can check the receipt attached to it instead of rebuilding the formalization, " +
        "which for a large Mathlib project takes hours and tens of gigabytes of memory."),
      h("li", {}, h("strong", {}, "Proofs you are not ready to publish. "),
        "The receipt shows that a proof exists, and when, without revealing it, much like the sealed " +
        "envelopes (", h("em", {}, "plis cachetés"), ") that mathematicians deposit with an academy. " +
        "You can later sign a claim that you are its author."),
    ),

    h("h2", {}, "How it works"),
    h("p", {},
      "You run a script in your Lean project. It exports your theorems with lean4export and checks them " +
      "with a Lean kernel (nanoda) inside the RISC Zero zero-knowledge virtual machine, on your own computer. " +
      "The virtual machine outputs a STARK proof that this checker accepted your declarations. That proof is " +
      "the receipt, about 220 KB. ",
      h("a", { href: "#/about" }, "More details"), "."),
    h("p", {},
      "Only the three standard axioms are allowed (", h("code", {}, "propext"), ", ", h("code", {}, "Quot.sound"),
      ", ", h("code", {}, "Classical.choice"), "). ", h("code", {}, "sorry"), ", ", h("code", {}, "native_decide"),
      " and new axioms are rejected. Mathlib is supported."),

    h("h2", {}, "Use it"),
    h("ul", {},
      h("li", {}, h("a", { href: "#/prove" }, "Prove a theorem"), ": one script, run in your project folder."),
      h("li", {}, h("a", { href: "#/verify" }, "Verify a receipt"), ": in this browser, nothing is uploaded."),
      h("li", {}, h("a", { href: "#/examples" }, "Examples"), ": receipts you can check right now."),
    ),

    h("h2", {}, "Status"),
    h("p", {},
      "Pre-alpha. The kernel that runs inside the virtual machine is not formally verified, and proving " +
      "takes from minutes to about an hour per theorem on a CPU. Do not rely on receipts for anything " +
      "important yet. The source code is on ", h("a", { href: "https://github.com/matteobusi/margin" }, "GitHub"), "."),
  );
}
