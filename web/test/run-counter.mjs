// Node test (SPEC.md §18.3 spike): loads examples/counter/dist/counter.plnt
// and a core exported with `plinth core export`, through the SAME host
// code the browser uses (web/plinth-web.js has no DOM dependency). Fires
// the Increment button's press event and asserts the tree text changes.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/counter
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/counter/dist/counter.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm } = await readPlnt(pltBytes);

  const app = new PlinthApp();
  let tree = buildTreeTracker(app);
  await app.load(coreBytes, appWasm, { log: () => {} });

  app.init([]);
  const textOf = (id) => tree.text.get(id);
  const findButtons = () => [...tree.listen.entries()];

  const before = [...tree.text.values()].join(" | ");
  assert.ok(before.includes("0"), `expected initial text to include "0", got: ${before}`);

  // Find a node listening for `press` (Event.press = 1) and fire it.
  let handler;
  for (const [id, byEvent] of tree.listen) {
    if (byEvent.has(1)) {
      handler = byEvent.get(1);
      break;
    }
  }
  assert.ok(handler !== undefined, "no press listener found in the counter's tree");

  app.onEvent({ kind: "ui", handler, event: 1, value: null });

  const after = [...tree.text.values()].join(" | ");
  assert.ok(after.includes("1"), `expected text to include "1" after press, got: ${after}`);
  assert.notEqual(before, after, "text did not change after the press event");

  console.log("run-counter.mjs: ok (before=%j after=%j)", before, after);
}

/** A tiny tree tracker: just enough to read text and find event listeners by node. */
function buildTreeTracker(app) {
  const text = new Map(); // node id -> string value (last Text op)
  const listen = new Map(); // node id -> Map<eventCode, handler>
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && op.prop === 4 /* value */) {
        text.set(op.id, String(op.value));
      }
    }
  };
  return { text, listen };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
