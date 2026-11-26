import { h } from "../dom";

export function renderAbout(root: HTMLElement) {
  root.append(
    h("h1", {}, "How it works"),
    h("p", { class: "subtitle" }, "What a receipt contains, and what you have to trust."),

    h("p", {},
      "On the prover's machine, Lean compiles the project and lean4export writes out the kernel declarations " +
      "of the chosen theorems. A Lean kernel type checker (nanoda) then runs on them inside the RISC Zero " +
      "zero-knowledge virtual machine. The checker rejects ", h("code", {}, "sorry"), ", ",
      h("code", {}, "native_decide"), " and any axiom other than the standard three. The virtual machine " +
      "produces a STARK proof that this exact program ran and accepted; that proof is the receipt."),

    h("h2", {}, "What the receipt reveals"),
    h("ul", {},
      h("li", {}, "The statements of the theorems claimed."),
      h("li", {}, "The definitions those statements use. Without them a statement could mean anything."),
      h("li", {}, "The axioms used, the version of Lean or Mathlib (as a hash, the environment root), and the prover's public key if they set one."),
    ),
    h("p", {}, "Proofs, lemmas and auxiliary definitions that the statements do not mention stay hidden."),

    h("h2", {}, "Checking the statement"),
    h("p", {},
      "Write the statement you care about in your own .lean file, with ", h("code", {}, "sorry"), " as proof, and run ",
      h("code", {}, "margin match"), " with the receipt. A match means that the receipt proves exactly your statement, " +
      "with exactly your definitions, over the official library. Kernel terms are compared byte by byte; names of " +
      "bound variables and notation play no role."),

    h("h2", {}, "What you have to trust"),
    h("ul", {},
      h("li", {}, "The checker named by the receipt's image ID. It is open source and its build is reproducible, " +
        "but it is not formally verified."),
      h("li", {}, "The soundness of RISC Zero's proof system."),
      h("li", {}, "The environment root: the library declarations and the axioms each one uses. Anyone can recompute it from the official release."),
      h("li", {}, "That the formal statement means what you think it means. That is why it is public."),
    ),
    h("p", {},
      "You do not have to trust the prover, this website or the Lean elaborator. Never take an image ID or an " +
      "environment root from the prover: use the published lists, or rebuild them."),

    h("h2", {}, "Related work"),
    h("p", {},
      "zkPi (Laufer, Ozdemir and Boneh, CCS 2024) builds a dedicated zkSNARK for Lean proofs. Margin instead runs " +
      "a complete kernel inside a general-purpose virtual machine, with transparent STARKs and no trusted setup. " +
      "More in the ", h("a", { href: "https://github.com/matteobusi/margin/blob/main/docs/security.md" }, "security notes"), "."),
  );
}
