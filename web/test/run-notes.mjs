// Node test (SPEC.md §8.5, §11, §18.3): `store.kv` backed by an in-memory
// `Map` (the Node default, `mapKvStore`). Adds a note in one `PlinthApp`
// instance, then starts a SECOND instance against the SAME backing map and
// checks the note is there (persistence across instances, same as two tabs
// sharing `localStorage`).
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/notes
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function newApp(coreBytes, appWasm, manifestText, sharedMap) {
  const app = new PlinthApp();
  const tree = buildTreeTracker(app);
  await app.load(coreBytes, appWasm, { log: () => {}, manifestText, sharedMap });
  app.init([]);
  return { app, tree };
}

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/notes/dist/notes.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm, manifestText } = await readPlnt(pltBytes);
  assert.ok(manifestText.includes('name = "store.kv"'), "expected the notes manifest to declare store.kv");

  const sharedMap = new Map(); // stands in for one localStorage origin shared by two instances

  const { app: app1, tree: tree1 } = await newApp(coreBytes, appWasm, manifestText, sharedMap);

  const titleHandler = findHandler(tree1, 2 /* Event.change */, "Title");
  const bodyHandler = findHandler(tree1, 2 /* Event.change */, "Body");
  const addHandler = findHandler(tree1, 1 /* Event.press */, "Add note");
  assert.ok(titleHandler !== undefined && bodyHandler !== undefined && addHandler !== undefined, "missing form handlers");

  app1.onEvent({ kind: "ui", handler: titleHandler, event: 2, value: "Groceries" });
  app1.onEvent({ kind: "ui", handler: bodyHandler, event: 2, value: "Bananas" });
  app1.onEvent({ kind: "ui", handler: addHandler, event: 1, value: null });

  const after1 = [...tree1.text.values(), ...tree1.rowTitles].join(" | ");
  assert.ok(after1.includes("Groceries"), `expected the new note's title in app1's tree, got: ${after1}`);
  assert.ok(sharedMap.size > 0, "expected the store write to land in the shared map");

  // A second instance against the SAME storage sees the persisted note.
  const { tree: tree2 } = await newApp(coreBytes, appWasm, manifestText, sharedMap);
  const after2 = [...tree2.text.values(), ...tree2.rowTitles].join(" | ");
  assert.ok(after2.includes("Groceries"), `expected the new note's title in app2's tree, got: ${after2}`);

  console.log("run-notes.mjs: ok (shared map had %d key(s), note found in both instances)", sharedMap.size);
}

/** Finds a node's handler id for `eventCode` whose last known label matches `label`. */
function findHandler(tree, eventCode, label) {
  for (const [id, byEvent] of tree.listen) {
    if (byEvent.has(eventCode) && tree.label.get(id) === label) {
      return byEvent.get(eventCode);
    }
  }
  return undefined;
}

/** A tiny tree tracker: enough to read text/labels/row titles and find event listeners. */
function buildTreeTracker(app) {
  const text = new Map(); // node id -> string value (last Text op)
  const label = new Map(); // node id -> last set-prop string value (label/title)
  const listen = new Map(); // node id -> Map<eventCode, handler>
  const rowTitles = new Set();
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && (op.prop === 3 /* Prop.label */ || op.prop === 1 /* Prop.title */)) {
        label.set(op.id, op.value);
        if (op.prop === 1) rowTitles.add(op.value);
      }
    }
  };
  return { text, label, listen, rowTitles };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
