// Writes the workspace version (from the root Cargo.toml) into every npm
// package.json: the package's own "version" and any "optionalDependencies"
// that point at another @plinth/* package.
//
//   node scripts/set-version.mjs            # use Cargo.toml's version
//   node scripts/set-version.mjs 0.2.0       # override, and write it back
//
// Run this before `npm-pack.mjs` or publishing, so all package.json files
// agree with the workspace version (npm-pack.mjs checks this and fails
// otherwise).
import { readFileSync, writeFileSync, readdirSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const cargoToml = join(root, "Cargo.toml");

function cargoVersion() {
  const text = readFileSync(cargoToml, "utf8");
  const m = text.match(/^version = "(.+)"/m);
  if (!m) throw new Error("Cargo.toml: no workspace version found");
  return m[1];
}

const override = process.argv[2];
const version = override ?? cargoVersion();

if (override) {
  const text = readFileSync(cargoToml, "utf8");
  const next = text.replace(/^version = "(.+)"/m, `version = "${override}"`);
  writeFileSync(cargoToml, next, "utf8");
}

const npmDir = join(root, "npm");
const pkgDirs = readdirSync(npmDir).filter((d) => existsSync(join(npmDir, d, "package.json")));

for (const dir of pkgDirs) {
  const path = join(npmDir, dir, "package.json");
  const pkg = JSON.parse(readFileSync(path, "utf8"));
  pkg.version = version;
  if (pkg.optionalDependencies) {
    for (const dep of Object.keys(pkg.optionalDependencies)) {
      if (dep.startsWith("@plinth/")) pkg.optionalDependencies[dep] = version;
    }
  }
  writeFileSync(path, JSON.stringify(pkg, null, 2) + "\n", "utf8");
  console.log(`npm/${dir}/package.json -> ${version}`);
}

console.log(`workspace version: ${version}`);
