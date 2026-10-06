// Node test (SPEC.md §18.3 spike): loads examples/todo/dist/todo.plnt and a
// core exported with `plinth core export`, through the same host code the
// browser uses. Adds a task via a TextField `change` event and the "Add
// task" button's `press` event, and asserts the task list and the counter
// text change.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/todo
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { ControlKind, Event } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/todo/dist/todo.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm } = await readPlnt(pltBytes);

  const app = new PlinthApp();
  const tree = buildTreeTracker(app);
  await app.load(coreBytes, appWasm, { log: () => {} });

  app.init([]);

  // The example seeds 3 tasks (examples/todo/app/model.ts), all open.
  const textValues = () => [...tree.text.values()].join(" | ");
  const before = textValues();
  assert.ok(before.includes("3 of 3 remaining"), `expected "3 of 3 remaining", got: ${before}`);
  assert.ok(!before.includes("Buy milk"));

  const textFieldId = [...tree.kind.entries()].find(([, k]) => k === ControlKind.textField)?.[0];
  assert.ok(textFieldId !== undefined, "no text-field node found");
  const changeHandler = tree.listen.get(textFieldId)?.get(Event.change);
  assert.ok(changeHandler !== undefined, "text-field has no change listener");

  app.onEvent({ kind: "ui", handler: changeHandler, event: Event.change, value: "Buy milk" });

  // The "Add task" button is the one labeled "Add task" (role=primary); the
  // other buttons on this screen are per-row "Delete" buttons.
  const labelOf = (id) => tree.text.get(id);
  const addButtonId = [...tree.kind.entries()]
    .filter(([, k]) => k === ControlKind.button)
    .map(([id]) => id)
    .find((id) => labelOf(id) === "Add task");
  assert.ok(addButtonId !== undefined, "no \"Add task\" button found");
  const pressHandler = tree.listen.get(addButtonId)?.get(Event.press);
  assert.ok(pressHandler !== undefined, "the Add task button has no press listener");

  app.onEvent({ kind: "ui", handler: pressHandler, event: Event.press, value: null });

  const after = textValues();
  assert.ok(after.includes("4 of 4 remaining"), `expected "4 of 4 remaining", got: ${after}`);
  assert.ok(after.includes("Buy milk"), `expected the task's title "Buy milk" in the tree, got: ${after}`);
  assert.notEqual(before, after);

  console.log("run-todo.mjs: ok");
}

function buildTreeTracker(app) {
  const text = new Map();
  const listen = new Map();
  const kind = new Map();
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "create") {
        kind.set(op.id, op.kind);
      } else if (op.op === "remove") {
        kind.delete(op.id);
        text.delete(op.id);
        listen.delete(op.id);
      } else if (op.op === "text") {
        text.set(op.id, String(op.value));
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && (op.prop === 1 /* title */ || op.prop === 3 /* label */ || op.prop === 4 /* value */)) {
        text.set(op.id, String(op.value));
      }
    }
  };
  return { text, listen, kind };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
