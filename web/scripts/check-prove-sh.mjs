// Checks that the environment table in public/prove.sh agrees with public/envs.json.
import fs from "node:fs";
const sh = fs.readFileSync(new URL("../public/prove.sh", import.meta.url), "utf8");
const envs = JSON.parse(fs.readFileSync(new URL("../public/envs.json", import.meta.url), "utf8"));
const table = sh.match(/^ENVS="\n([\s\S]*?)^"/m)?.[1];
if (!table) throw new Error("prove.sh: ENVS table not found");
const rows = table.trim().split("\n").map((l) => l.split("|"));
const fail = (m) => { console.error("check-prove-sh: " + m); process.exit(1); };
if (rows.length !== envs.length) fail(`prove.sh lists ${rows.length} environments, envs.json ${envs.length}`);
for (const [id, lean, mathlib, root] of rows) {
  const e = envs.find((x) => x.env_root === root);
  if (!e) fail(`${id}: root ${root} not in envs.json`);
  if (e.lean !== lean) fail(`${id}: toolchain ${lean} vs envs.json ${e.lean}`);
  if ((e.mathlib ?? "-") !== mathlib) fail(`${id}: Mathlib ${mathlib} vs envs.json ${e.mathlib ?? "-"}`);
}
console.log(`check-prove-sh: ${rows.length} environments agree`);
