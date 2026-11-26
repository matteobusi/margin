import { h } from "../dom";
import { resultView } from "../result";
import { verifyPublished } from "../verifier";

interface Example {
  title: string;
  description: string;
  receipt: string;
  statement: string;
  claims: string[];
  authorship?: string;
}

export function renderExamples(root: HTMLElement) {
  const list = h("div", {}, h("p", { class: "note" }, "Loading…"));
  root.append(
    h("h1", {}, "Examples"),
    h("p", { class: "subtitle" }, "Receipts made with the published checker."),
    list,
  );
  fetch("examples/index.json", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : []))
    .then((items: Example[]) => {
      list.replaceChildren();
      if (items.length === 0) {
        list.append(h("p", {}, "The examples are being re-made with the current checker and will be back shortly."));
        return;
      }
      for (const ex of items) list.append(example(ex));
    })
    .catch(() => list.replaceChildren(h("p", { class: "note" }, "Could not load the examples.")));
}

function example(ex: Example) {
  const out = h("div", { "aria-live": "polite" });
  const button = h("button", { class: "link", type: "button" }, "verify it here");
  button.addEventListener("click", async () => {
    button.setAttribute("disabled", "");
    out.replaceChildren(h("p", { class: "note" }, "Checking…"));
    try {
      const bytes = (p: string) => fetch(`examples/${p}`).then((r) => r.arrayBuffer()).then((b) => new Uint8Array(b));
      const [receipt, claim] = await Promise.all([
        bytes(ex.receipt),
        ex.authorship ? fetch(`examples/${ex.authorship}`).then((r) => r.text()) : Promise.resolve(undefined),
      ]);
      out.replaceChildren(resultView(await verifyPublished(receipt, claim)));
    } catch (e) {
      out.replaceChildren(h("p", { class: "status bad" }, String(e instanceof Error ? e.message : e)));
    } finally {
      button.removeAttribute("disabled");
    }
  });
  const links: (Node | string)[] = [
    button, " · ",
    h("a", { href: `examples/${ex.receipt}`, download: "" }, "download the receipt"), " · ",
    h("a", { href: `examples/${ex.statement}`, target: "_blank" }, "statement"),
  ];
  if (ex.authorship) links.push(" · ", h("a", { href: `examples/${ex.authorship}`, target: "_blank" }, "authorship claim"));
  return h("section", { class: "example" },
    h("h2", {}, ex.title),
    h("p", {}, ex.description),
    h("p", { class: "small" }, ...links),
    out,
  );
}
