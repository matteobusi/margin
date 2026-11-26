import { BRAND } from "../brand";
import { h } from "../dom";

export function renderHome(root: HTMLElement) {
  root.append(
    h("section", { class: "hero" },
      h("p", { class: "badge" }, BRAND.status),
      h("h1", {}, BRAND.tagline),
      h("p", { class: "lead" },
        "Prove that Lean accepts your proof, without showing the proof. Run one script in your Lean 4 project: it type-checks your proof inside a zero-knowledge virtual machine " +
        "and gives you a receipt. The receipt shows your theorem statements and hides your proofs. " +
        "Anyone can verify it in a browser. Mathlib is supported."),
      h("div", { class: "cta" },
        h("a", { href: "#/prove", class: "button primary" }, "Prove a theorem"),
        h("a", { href: "#/verify", class: "button" }, "Verify a receipt"),
        h("a", { href: "#/examples", class: "button" }, "See examples")),
    ),
    h("section", { class: "flow" },
      step("1", "Prove, on your machine",
        "Download prove.sh and run it in your project folder. It exports your theorems, checks them, and proves them in the zkVM. Your proofs never leave your computer."),
      step("2", "Publish the receipt",
        "Put the receipt (about 220 KB) on your page or next to your paper. It contains the statements, the definitions they use, the axioms, and your pseudonymous owner key."),
      step("3", "Anyone verifies",
        "In a browser or with the CLI, in a fraction of a second, against the published checker and library versions. Later, you can sign a claim to prove you are the author."),
    ),
    h("section", { class: "grid3" },
      card("Private", "Proofs never leave your machine. The receipt is a zero-knowledge STARK: it reveals nothing about the proof."),
      card("Transparent", "No trusted setup. Open-source checker, reproducible build, published image ID and library roots."),
      card("Strict", "No sorry, no native_decide, no custom axioms. Only propext, Quot.sound and Classical.choice."),
    ),
    h("section", { class: "warn" },
      h("strong", {}, "Status: "),
      "pre-alpha. The type checker in the zkVM (nanoda) is not formally verified. " +
      "Do not rely on receipts for anything important yet."),
  );
}

function step(n: string, title: string, text: string) {
  return h("div", { class: "flow-step" }, h("span", { class: "num" }, n), h("h3", {}, title), h("p", {}, text));
}

function card(title: string, text: string) {
  return h("div", { class: "card" }, h("h3", {}, title), h("p", {}, text));
}
