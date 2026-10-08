// Generate the license list in docs/DEPENDENCIES.md (§5).
//
//   node scripts/gen-license-list.mjs           # print the list
//   node scripts/gen-license-list.mjs --write   # replace it in docs/DEPENDENCIES.md
//
// It reads `cargo deny list` (the same graph and targets as deny.toml) and
// `cargo metadata` (for the first-party crates: Plinth and gpui-ce).
// Run it from the repository root after a change to Cargo.lock.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const run = (args) =>
  execFileSync("cargo", args, { encoding: "utf8", maxBuffer: 256 << 20, stdio: ["ignore", "pipe", "ignore"] });

// Third-party and first-party crates, from cargo-deny.
const byCrate = JSON.parse(run(["deny", "list", "-l", "crate", "-f", "json"]));
const meta = JSON.parse(run(["metadata", "--format-version", "1", "--locked"]));

// A crate id from `cargo deny list` is "name version source"; path crates
// have a "path+file://" source.
const firstParty = new Map(); // name -> license expression
for (const p of meta.packages) {
  if (p.source === null) firstParty.set(`${p.name} ${p.version}`, p.license ?? "(none)");
}

const counts = new Map(); // license -> number of crates
const exprs = new Map(); // license set (as "A OR B") -> number of crates
let total = 0;
for (const [id, { licenses }] of Object.entries(byCrate)) {
  const [name, version] = id.split(" ");
  if (firstParty.has(`${name} ${version}`)) continue;
  total++;
  for (const l of licenses) counts.set(l, (counts.get(l) ?? 0) + 1);
  const key = [...licenses].sort().join(" / ") || "(none)";
  exprs.set(key, (exprs.get(key) ?? 0) + 1);
}

const sorted = (m) => [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
const out = [];
out.push(`Third-party crates in the checked graph: **${total}**.`);
out.push("");
out.push("Crates whose license expression names each license (a crate that names two licenses counts once for each):");
out.push("");
out.push("| License | Crates |");
out.push("|---|---|");
for (const [l, n] of sorted(counts)) out.push(`| ${l} | ${n} |`);
out.push("");
out.push("Crates by license set (all licenses that the license expression of one crate names, joined by OR or AND):");
out.push("");
out.push("| Licenses in the expression of one crate | Crates |");
out.push("|---|---|");
for (const [l, n] of sorted(exprs)) out.push(`| ${l} | ${n} |`);
out.push("");
out.push("First-party crates (path dependencies; not counted above):");
out.push("");
out.push("| License | Crates |");
out.push("|---|---|");
const fp = new Map();
for (const [id, lic] of firstParty) {
  const name = id.split(" ")[0];
  const group = name.startsWith("plinth") || name.endsWith("-rs") ? "Plinth" : "gpui-ce";
  const key = `${lic} (${group})`;
  fp.set(key, [...(fp.get(key) ?? []), name]);
}
for (const [k, names] of [...fp.entries()].sort()) out.push(`| ${k} | ${names.sort().join(", ")} |`);
const text = out.join("\n");

if (process.argv.includes("--write")) {
  const path = "docs/DEPENDENCIES.md";
  const begin = "<!-- license-list:begin -->";
  const end = "<!-- license-list:end -->";
  const doc = readFileSync(path, "utf8");
  const a = doc.indexOf(begin);
  const b = doc.indexOf(end);
  if (a < 0 || b < a) throw new Error(`${path}: markers not found`);
  writeFileSync(path, doc.slice(0, a + begin.length) + "\n" + text + "\n" + doc.slice(b));
  console.log(`updated ${path}`);
} else {
  console.log(text);
}
