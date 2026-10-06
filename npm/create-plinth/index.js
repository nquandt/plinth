#!/usr/bin/env node
// `npm create plinth@latest [dir]`: makes a new Plinth app with `plinth new`.
// It uses PLINTH_BINARY, an installed @plinth/cli, or npx, in that order.
"use strict";

const { spawnSync } = require("node:child_process");
const readline = require("node:readline");

const VERSION = require("./package.json").version;

function run(cmd, args) {
  const r = spawnSync(cmd, args, { stdio: "inherit", shell: process.platform === "win32" && cmd === "npx" });
  if (r.error) throw r.error;
  return r.status ?? 1;
}

function plinth(args) {
  if (process.env.PLINTH_BINARY) return run(process.env.PLINTH_BINARY, args);
  try {
    const bin = require.resolve("@plinth/cli/bin/plinth.js");
    return run(process.execPath, [bin, ...args]);
  } catch {
    return run("npx", ["--yes", `@plinth/cli@${VERSION}`, ...args]);
  }
}

async function main() {
  let dir = process.argv[2];
  if (!dir) {
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    dir = await new Promise((resolve) => rl.question("Project name (my-app): ", (a) => resolve(a.trim() || "my-app")));
    rl.close();
  }
  process.exit(plinth(["new", dir]));
}

main().catch((e) => {
  console.error(`create-plinth: ${e.message}`);
  process.exit(1);
});
