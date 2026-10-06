// Makes the npm tarballs in target/npm (SPEC.md §13.1).
//
//   cargo build --release -p plinth-cli
//   node scripts/npm-pack.mjs
//
// Only the binary of the current platform is packed by default; CI packs
// the others by passing --platform and --binary explicitly (one job per
// platform, since each binary is built on its own runner):
//
//   node scripts/npm-pack.mjs --platform linux-x64 --binary /path/to/plinth
//   node scripts/npm-pack.mjs --meta           # @plinth/cli + create-plinth only
//
// --platform takes a "<os>-<arch>" pair in Node's process.platform/arch
// vocabulary (win32-x64, linux-x64, darwin-x64, darwin-arm64). --binary
// defaults to target/release/<exe> for the current platform.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const version = readFileSync(join(root, "Cargo.toml"), "utf8").match(/^version = "(.+)"/m)[1];

function arg(name) {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? undefined : process.argv[i + 1];
}

const metaOnly = process.argv.includes("--meta");
const platform = arg("platform") ?? `${process.platform}-${process.arch}`;
const exe = platform.startsWith("win32") ? "plinth.exe" : "plinth";
const binary = arg("binary") ?? join(root, "target", "release", exe);
const platformDir = join(root, "npm", `cli-${platform}`);
const out = join(root, "target", "npm");
mkdirSync(out, { recursive: true });

function checkVersion(dir) {
  const pkg = JSON.parse(readFileSync(join(root, "npm", dir, "package.json"), "utf8"));
  if (pkg.version !== version) {
    throw new Error(
      `npm/${dir}/package.json has version ${pkg.version}, but Cargo.toml has ${version}. Run scripts/set-version.mjs first.`,
    );
  }
}

function pack(dir) {
  execFileSync("npm", ["pack", "--pack-destination", out], {
    cwd: join(root, "npm", dir),
    stdio: "inherit",
    shell: process.platform === "win32",
  });
}

if (!metaOnly) {
  if (!existsSync(binary)) throw new Error(`build first: cargo build --release -p plinth-cli (${binary} is missing)`);
  if (!existsSync(platformDir)) throw new Error(`no npm package for ${platform} (npm/cli-${platform})`);
  checkVersion(`cli-${platform}`);

  copyFileSync(binary, join(platformDir, exe));
  // Windows: `plinthw.exe` runs `plinth.exe` with no console window, for
  // shortcuts and plinth:// links (docs/HUB.md §10).
  if (platform.startsWith("win32")) {
    const launcher = join(dirname(binary), "plinthw.exe");
    if (!existsSync(launcher)) throw new Error(`build first: cargo build --release -p plinth-cli (${launcher} is missing)`);
    copyFileSync(launcher, join(platformDir, "plinthw.exe"));
  }
  pack(`cli-${platform}`);
  console.log(`packed npm/cli-${platform} (binary: ${binary}) -> ${out}`);
}

if (metaOnly || !arg("platform")) {
  // Packed once: the typings and shim (@plinth/cli) and the scaffolder
  // (create-plinth) do not vary per platform.
  checkVersion("cli");
  checkVersion("create-plinth");

  mkdirSync(join(root, "npm", "cli", "types"), { recursive: true });
  for (const f of ["lib.d.ts", "ui.d.ts", "core.d.ts"]) {
    copyFileSync(join(root, "std", f), join(root, "npm", "cli", "types", f));
  }

  pack("cli");
  pack("create-plinth");
  console.log(`packed npm/cli and npm/create-plinth -> ${out}`);
}
