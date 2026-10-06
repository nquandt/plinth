// Makes the npm tarballs in target/npm (SPEC.md §13.1).
//
//   cargo build --release -p plinth-cli
//   node scripts/npm-pack.mjs
//
// Only the binary of the current platform is packed; CI packs the others.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, existsSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const version = readFileSync(join(root, "Cargo.toml"), "utf8").match(/^version = "(.+)"/m)[1];
const platform = `${process.platform}-${process.arch}`;
const exe = process.platform === "win32" ? "plinth.exe" : "plinth";
const binary = join(root, "target", "release", exe);
const platformDir = join(root, "npm", `cli-${platform}`);

if (!existsSync(binary)) throw new Error(`build first: cargo build --release -p plinth-cli (${binary} is missing)`);
if (!existsSync(platformDir)) throw new Error(`no npm package for ${platform} (npm/cli-${platform})`);
for (const dir of ["cli", `cli-${platform}`, "create-plinth"]) {
  const pkg = JSON.parse(readFileSync(join(root, "npm", dir, "package.json"), "utf8"));
  if (pkg.version !== version) throw new Error(`npm/${dir}/package.json has version ${pkg.version}, but Cargo.toml has ${version}`);
}

copyFileSync(binary, join(platformDir, exe));
// The typings ship with @plinth/cli too, for editors.
mkdirSync(join(root, "npm", "cli", "types"), { recursive: true });
for (const f of ["lib.d.ts", "ui.d.ts", "core.d.ts"]) copyFileSync(join(root, "std", f), join(root, "npm", "cli", "types", f));

const out = join(root, "target", "npm");
mkdirSync(out, { recursive: true });
for (const dir of ["cli", `cli-${platform}`, "create-plinth"]) {
  execFileSync("npm", ["pack", "--pack-destination", out], { cwd: join(root, "npm", dir), stdio: "inherit", shell: process.platform === "win32" });
}
console.log(`tarballs in ${out}`);
