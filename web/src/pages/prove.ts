import { h } from "../dom";
import { commandBlock, setCommand } from "../result";

interface Env {
  name: string;
  lean?: string;
  mathlib?: string;
}

export function renderProve(root: HTMLElement) {

  // ---- command builder ----
  const module = input("module", "MyProject.Basic");
  const theorems = input("theorems", "my_theorem other_theorem");
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
    setCommand(claim, `~/.margin/bin/margin claim --note "Your Name" margin-out/${thms[0] ?? "my_theorem"}.receipt ~/.margin/owner.key > claim.json`);
  };
  for (const el of [module, theorems, anonymous, checkOnly]) el.addEventListener("input", update);
  update();

  // ---- supported versions ----
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
    h("p", { class: "lead" },
      "Proving runs on your own computer, with one script. Your proofs never leave your machine: " +
      "the script downloads tools, never uploads anything."),

    h("h2", {}, "1. Check that you have"),
    h("ul", { class: "checklist" },
      h("li", {}, "Linux (x86-64) or macOS (Apple silicon), with about ", h("strong", {}, "12 GB of RAM"), "."),
      h("li", {}, "A Lean 4 project built with Lake, using one of the supported versions below. Its theorems must compile without ",
        h("code", {}, "sorry"), "."),
      h("li", {}, h("code", {}, "git"), ", ", h("code", {}, "curl"), " and ", h("code", {}, "elan"), " (you have them if you use Lean)."),
    ),
    h("table", { class: "versions" },
      h("thead", {}, h("tr", {}, h("th", {}, "lean-toolchain"), h("th", {}, "Library"))),
      versions),
    h("p", { class: "note" }, "Another version? It needs a published environment; open an issue."),

    h("h2", {}, "2. Run the script in your project folder"),
    h("div", { class: "builder" },
      h("label", { for: "module" }, "Module that contains the theorems"), module,
      h("label", { for: "theorems" }, "Theorems to prove (space-separated)"), theorems,
      h("label", { class: "check" }, checkOnly, " Pre-check only (fast, no proof)"),
      h("label", { class: "check" }, anonymous, " Anonymous (no owner key, authorship can never be claimed)"),
    ),
    h("p", {}, "Download the script once:"),
    download,
    h("p", {}, "Then run, from the folder that contains ", h("code", {}, "lakefile.lean"), " or ", h("code", {}, "lakefile.toml"), ":"),
    run,
    h("p", { class: "note" }, "Read it first if you like: ", h("a", { href: "prove.sh", target: "_blank" }, "prove.sh"),
      " is a plain shell script. Run ", h("code", {}, "bash prove.sh --help"), " for all options."),

    h("h2", {}, "What the script does"),
    h("table", { class: "steps-table" },
      h("tbody", {},
        row("Checks your project", "Reads lean-toolchain and lake-manifest.json, and picks the matching environment.", "seconds"),
        row("Downloads margin", "The prover CLI for your platform, checked against its SHA-256. Stored in ~/.margin.", "once"),
        row("Builds lean4export", "The official Lean exporter, for your Lean version.", "once, a few minutes"),
        row("Gets the library cache", "Downloads it, or builds it from your installed Mathlib. Checked against the published root.", "once, up to 30 min"),
        row("Creates your owner key", "~/.margin/owner.key. Its public half goes in the receipt, so you can claim authorship later.", "once"),
        row("Pre-checks", "Exports your theorems and runs the exact checker natively. Rejects sorry, native_decide and custom axioms. Estimates the proving time.", "about 2 min"),
        row("Proves", "Runs the checker in the RISC Zero zkVM and writes margin-out/<theorem>.receipt.", "minutes to about an hour"),
      )),

    h("h2", {}, "3. Publish"),
    h("ul", {},
      h("li", {}, "Publish ", h("code", {}, "<theorem>.receipt"), " wherever you like: your web page, a paper, a repository. " +
        "It shows your statements and hides your proofs. Anyone can check it on the ",
        h("a", { href: "#/verify" }, "Verify page"), "."),
      h("li", {}, "Also publish a statement file: your theorems with ", h("code", {}, ":= by sorry"),
        ". Readers can then run ", h("code", {}, "margin match"), " against their own copy of the statement."),
      h("li", {}, "To claim authorship, now or years later, sign the receipt with your owner key:"),
    ),
    claim,
    h("p", { class: "note" }, "Publish claim.json next to the receipt (anyone can check it on the Verify page). Keep ~/.margin/owner.key secret and backed up: " +
      "without it nobody, including you, can claim the receipt."),

    h("details", {},
      h("summary", {}, "Using margin directly"),
      h("p", {}, "The script only wraps the margin command-line tool. Its main commands:"),
      h("pre", { class: "output" },
        "margin keygen  owner.key\n" +
        "margin run     --key owner.key <env.cache> <Module> <out.receipt> <theorem>...\n" +
        "margin verify  [--claim claim.json] <receipt>\n" +
        "margin match   <receipt> <env.cache> <statement.ndjson> <theorem>...\n" +
        "margin claim   --note TEXT <receipt> owner.key"),
      h("p", {}, "Binaries and source: see the project repository."),
    ),
  );
}

function input(id: string, placeholder: string) {
  return h("input", { id, type: "text", placeholder, spellcheck: "false", autocomplete: "off" });
}

function row(what: string, detail: string, time: string) {
  return h("tr", {}, h("th", { scope: "row" }, what), h("td", {}, detail), h("td", { class: "note" }, time));
}
