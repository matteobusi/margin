import { h } from "../dom";

export function renderAbout(root: HTMLElement) {
  root.append(
    h("h1", {}, "How it works"),
    h("ol", { class: "how" },
      h("li", {}, "On your machine, Lean compiles your project and lean4export writes out the kernel declarations of your theorems."),
      h("li", {}, "A Lean kernel type checker runs inside the RISC Zero zkVM, on your machine."),
      h("li", {}, "The checker rejects sorry, native_decide and any custom axiom."),
      h("li", {}, "The zkVM produces a zero-knowledge STARK receipt. It commits to the theorem " +
        "statements, the definitions they use, the trusted environment root and the axioms used."),
      h("li", {}, "Anyone can verify the receipt against the published checker image IDs."),
    ),
    h("h2", {}, "What is public"),
    h("ul", {},
      h("li", {}, "The statements of the theorems you claim."),
      h("li", {}, "The definitions those statements use (otherwise a statement could mean anything)."),
      h("li", {}, "The axioms used, the environment root and your public key."),
    ),
    h("h2", {}, "How to check that it proves your statement"),
    h("ol", {},
      h("li", {}, "Write the statement you care about in your own .lean file, with proof sorry."),
      h("li", {}, "Export it with lean4export (margin export), and run margin match with the receipt."),
      h("li", {}, "MATCH means: the receipt proves exactly your statement, with exactly your definitions, " +
        "over the official Lean library. Names and pretty-printing are never compared; kernel terms are, byte by byte."),
    ),
    h("p", {}, "Do not accept an image ID or an environment root from the prover. Use the published lists " +
      "(checkers.json, envs.json), or rebuild them yourself: both are reproducible."),
    h("h2", {}, "What you must trust"),
    h("ul", {},
      h("li", {}, "The type checker that the image ID names. It is open source and rebuilds reproducibly. " +
        "It is not formally verified yet."),
      h("li", {}, "The soundness of RISC Zero's STARK proof system."),
      h("li", {}, "The environment root, including the axiom set recorded for each library declaration. " +
        "Anyone can recompute it from the official Lean release."),
      h("li", {}, "That the formal statement and its definitions mean what you think. They are public so experts can review them."),
    ),
    h("p", {}, "You do not need to trust the prover, this website, its server or the Lean elaborator."),
    h("h2", {}, "Prior work"),
    h("p", {},
      "zkPi (Laufer, Ozdemir, Boneh, CCS 2024) builds a custom zkSNARK for Lean proofs. " +
      "This project instead runs a complete kernel in a general-purpose zkVM, with transparent STARKs."),
  );
}
