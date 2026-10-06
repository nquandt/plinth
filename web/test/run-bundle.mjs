// Node test of the bundle `web/plinth.js` (SPEC.md §10.3, made by
// `scripts/gen-plinth-js.mjs`): the bundle loads with no DOM, it is up to
// date with the files in web/, and its `PlinthApp` runs the counter app.
// The browser side (`<plinth-app>`, the frame) is in run-a11y.mjs.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/counter
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt, parseManifest, base64Bytes, defaultCoreUrl, Tree } from "../plinth.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

execFileSync(process.execPath, [path.join(root, "scripts/gen-plinth-js.mjs"), "--check"], { stdio: "inherit" });

assert.deepEqual([...base64Bytes(" AAEC\n/w== ")], [0, 1, 2, 255]);
assert.equal(defaultCoreUrl("plinth-core/1.3", "https://example.com/app/plinth.js"), "https://example.com/app/plinth-core-1.3.wasm");
assert.equal(defaultCoreUrl("", "https://example.com/plinth.js"), "https://example.com/plinth-core-1.0.wasm");

const pkg = new Uint8Array(readFileSync(path.join(root, "examples/counter/dist/counter.plnt")));
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const { appWasm, manifestText } = await readPlnt(pkg);
const manifest = parseManifest(manifestText);
assert.equal(manifest.id, "dev.plinth.examples.counter");
assert.ok(manifest.name.length > 0, "the manifest has a name");
assert.match(manifest.runtime, /^plinth-core\/1\.\d+$/);

const app = new PlinthApp();
const tree = new Tree();
app.onCommit = (ops) => tree.apply(ops);
await app.load(core, appWasm, { log: () => {}, manifestText });
app.init([]);
const texts = () => [...tree.nodes.values()].map((n) => n.text ?? "").join(" | ");
const press = [...tree.nodes.values()].find((n) => n.listeners.has(1)); // Event.press = 1
assert.ok(press, "the counter has a node with a press listener");
const before = texts();
app.onEvent({ kind: "ui", handler: press.listeners.get(1), event: 1, value: null });
assert.notEqual(texts(), before, "a press changes the text");
console.log("run-bundle.mjs: ok (plinth.js %d KiB)", Math.round(readFileSync(path.join(root, "web/plinth.js")).length / 1024));
