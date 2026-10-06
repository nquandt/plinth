#!/usr/bin/env node
// Runs the native `plinth` binary for this platform. The binary comes from
// the optional dependency @plinth/cli-<platform>-<arch> (the esbuild
// pattern), so `npm install` needs no build step. PLINTH_BINARY overrides it.
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

function binaryPath() {
  if (process.env.PLINTH_BINARY) return process.env.PLINTH_BINARY;
  const pkg = `@plinth/cli-${process.platform}-${process.arch}`;
  const exe = process.platform === "win32" ? "plinth.exe" : "plinth";
  try {
    return path.join(path.dirname(require.resolve(`${pkg}/package.json`)), exe);
  } catch {
    console.error(
      `plinth: there is no binary for ${process.platform}-${process.arch}.\n` +
        `The package ${pkg} is not installed. Supported platforms: win32-x64.\n` +
        `Set PLINTH_BINARY to a plinth binary to use your own build.`,
    );
    process.exit(1);
  }
}

const result = spawnSync(binaryPath(), process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`plinth: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
