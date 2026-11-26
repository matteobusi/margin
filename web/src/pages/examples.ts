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
  const list = h("div", { class: "examples" }, h("p", { class: "note" }, "Loading…"));
  root.append(
    h("h1", {}, "Examples"),
    h("p", { class: "lead" }, "Real receipts, made with the published checker. Verify them here, or download them."),
    list,
  );
  fetch("examples/index.json", { cache: "no-store" })
    .then((r) => (r.ok ? r.json() : []))
    .then((items: Example[]) => {
      list.replaceChildren();
      if (items.length === 0) {
        list.append(h("p", { class: "note" }, "No published examples yet."));
        return;
      }
      for (const ex of items) list.append(card(ex));
    })
    .catch(() => list.replaceChildren(h("p", { class: "note" }, "Could not load examples.")));
}

function card(ex: Example) {
  const out = h("div", { "aria-live": "polite" });
  const button = h("button", { class: "button primary", type: "button" }, "Verify in this browser");
  button.addEventListener("click", async () => {
    button.setAttribute("disabled", "");
    out.replaceChildren(h("p", { class: "note" }, "Checking…"));
    try {
      const fetchBytes = (p: string) => fetch(`examples/${p}`).then((r) => r.arrayBuffer()).then((b) => new Uint8Array(b));
      const [receipt, claim] = await Promise.all([
        fetchBytes(ex.receipt),
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
    h("a", { href: `examples/${ex.receipt}`, download: "" }, "receipt"), " · ",
    h("a", { href: `examples/${ex.statement}`, target: "_blank" }, "statement file"),
  ];
  if (ex.authorship) links.push(" · ", h("a", { href: `examples/${ex.authorship}`, target: "_blank" }, "authorship claim"));
  return h("div", { class: "card" },
    h("h3", {}, ex.title),
    h("p", {}, ex.description),
    h("p", { class: "links" }, ...links),
    h("div", { class: "cta" }, button),
    out,
  );
}
