import { h } from "../dom";
import { commandBlock, setCommand } from "../result";

interface Env {
  name: string;
  lean?: string;
  mathlib?: string;
}

export function renderProve(root: HTMLElement) {
  // Command builder.
  const module = input("module", "MyProject.Basic");
  const theorems = input("theorems", "my_theorem");
  const anonymous = h("input", { type: "checkbox", id: "anon" });
  const checkOnly = h("input", { type: "checkbox", id: "checkonly" });
  const download = commandBlock("");
  const run = commandBlock("");
  const claim = commandBlock("");
  const update = () => {
    const mod = module.value.trim() || "MyProject.Basic";
    const thms = theorems.value.trim().split(/[\s,]+/).filter(Boolean);
    const flags = [checkOnly.checked ? "--check-only" : "", anonymous.checked ? "--anonymous" : ""].filter(Boolean);
    setCommand(download, `curl -fsSLO ${new URL("prove.sh", document.baseURI).href}`);
    setCommand(run, ["bash prove.sh", ...flags, mod, ...(thms.length ? thms : ["my_theorem"])].join(" "));
    setCommand(claim, `~/.margin/bin/margin claim --note "Your Name" \\\n  margin-out/${thms[0] ?? "my_theorem"}.receipt ~/.margin/owner.key > claim.json`);
  };
  for (const el of [module, theorems, anonymous, checkOnly]) el.addEventListener("input", update);
  update();

  // Supported versions, from envs.json.
  const versions = h("tbody", {}, h("tr", {}, h("td", { colspan: "2", class: "note" }, "Loading…")));
  fetch("envs.json", { cache: "no-store" })
    .then((r) => r.json())
    .then((envs: Env[]) => {
      versions.replaceChildren(...envs.map((e) => h("tr", {},
        h("td", {}, h("code", {}, e.lean ?? "?")),
        h("td", {}, e.mathlib ? h("span", {}, "Mathlib ", h("code", {}, e.mathlib.slice(0, 7))) : "core Lean only"),
      )));
    })
    .catch(() => versions.replaceChildren(h("tr", {}, h("td", { colspan: "2" }, "Could not load envs.json."))));

  root.append(
    h("h1", {}, "Prove a theorem"),
    h("p", { class: "subtitle" }, "On your own computer, with one script."),

    h("p", {},
      "You need Linux (x86-64) or a Mac with Apple silicon, about 12 GB of memory, and a Lean 4 project built " +
      "with Lake whose theorems compile without ", h("code", {}, "sorry"), ". The project must use one of the " +
      "versions below; others need a published environment, so ",
      h("a", { href: "https://github.com/matteobusi/margin/issues" }, "open an issue"), " if yours is missing."),
    h("table", {},
      h("thead", {}, h("tr", {}, h("th", {}, "lean-toolchain"), h("th", {}, "library"))),
      versions),

    h("h2", {}, "Run the script"),
    h("p", {}, "Fill in your module and theorems to get the commands:"),
    h("div", { class: "builder" },
      h("label", { for: "module" }, "Module"), module,
      h("label", { for: "theorems" }, "Theorems"), theorems,
      h("label", { class: "check" }, checkOnly, "Only pre-check (a couple of minutes, no proof)"),
      h("label", { class: "check" }, anonymous, "Anonymous: no owner key, so nobody can ever claim authorship"),
    ),
    h("p", {}, "Download the script, then run it in the folder that contains your lakefile:"),
    download,
    run,
    h("p", {},
      "The script is plain bash; ", h("a", { href: "prove.sh", target: "_blank" }, "read it"),
      " before running it if you like. It only downloads; nothing about your project leaves your computer."),

    h("h2", {}, "What happens"),
    h("ol", {},
      h("li", {}, "It reads ", h("code", {}, "lean-toolchain"), " and ", h("code", {}, "lake-manifest.json"),
        " to find your Lean and Mathlib versions."),
      h("li", {}, "The first time only, it downloads the margin tool (checking its SHA-256), builds lean4export " +
        "for your Lean version, downloads the library data for your Mathlib version (1.8 GB) and creates an owner key " +
        "in ", h("code", {}, "~/.margin"), "."),
      h("li", {}, "It builds your project, exports the theorems and runs the same checker natively. This rejects ",
        h("code", {}, "sorry"), ", ", h("code", {}, "native_decide"), " and new axioms, and estimates the proving time."),
      h("li", {}, "It asks before proving, then proves. Expect minutes for small theorems and about an hour for a " +
        "typical Mathlib result on a 16-core machine. The receipt goes to ", h("code", {}, "margin-out/"), "."),
    ),

    h("h2", {}, "Publish"),
    h("p", {},
      "Put the receipt wherever you like: next to a paper, on your web page, in a repository. It shows the " +
      "statements and hides the proofs. Add a statement file, your theorems with ", h("code", {}, ":= by sorry"),
      ", so that readers can check with ", h("code", {}, "margin match"), " that the receipt proves exactly what you claim."),
    h("p", {}, "To show that you are the author, now or years later, sign the receipt with your owner key and publish the result:"),
    claim,
    h("p", {},
      "Keep ", h("code", {}, "~/.margin/owner.key"), " secret and backed up. Without it nobody, you included, can claim the receipt."),

    h("details", {},
      h("summary", {}, "Using the margin tool directly"),
      h("p", {}, "The script is a wrapper around a few commands:"),
      h("pre", {},
        "margin keygen  owner.key\n" +
        "margin run     --key owner.key <env.cache> <Module> <out.receipt> <theorem>...\n" +
        "margin verify  [--claim claim.json] <receipt>\n" +
        "margin match   <receipt> <env.cache> <statement.ndjson> <theorem>...\n" +
        "margin claim   --note TEXT <receipt> owner.key"),
      h("p", {}, "Binaries are on the ", h("a", { href: "https://github.com/matteobusi/margin/releases" }, "releases page"), "."),
    ),
  );
}

function input(id: string, placeholder: string) {
  return h("input", { id, type: "text", placeholder, spellcheck: "false", autocomplete: "off" });
}
