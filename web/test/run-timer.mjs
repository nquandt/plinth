// Node test (SPEC.md §8.4, §18.3): loads examples/timer/dist/timer.plnt and
// checks that a fired `setInterval` timer is delivered back into the app as
// a `timer` event through `on-event`, and that the resulting re-render
// changes the heading text (PlinthApp's default `scheduleTimerEvent`, wired
// in plinth-web.js).
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/timer
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/timer/dist/timer.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm } = await readPlnt(pltBytes);

  const app = new PlinthApp();
  const tree = buildTreeTracker(app);
  await app.load(coreBytes, appWasm, { log: () => {} });

  app.init([]);

  const headingText = () => [...tree.text.values()].find((v) => /s$/.test(v)) ?? [...tree.text.values()].join(" | ");
  const before = headingText();
  assert.ok(before.includes("0.0"), `expected initial heading to show 0.0s, got: ${before}`);

  // Fire the Start button's press handler (same shape as run-counter.mjs).
  const pressHandler = findHandler(tree, 1 /* Event.press */, "Start");
  assert.ok(pressHandler !== undefined, "no Start press listener found");
  app.onEvent({ kind: "ui", handler: pressHandler, event: 1, value: null });

  // The app's `setInterval` is a real JS timer (plinth-web.js host import).
  // Wait for a couple of ticks (TICK_MS = 100) and let the real timer fire;
  // PlinthApp's default scheduleTimerEvent turns each fire into a `timer`
  // event, which runs the app's callback and re-renders.
  await new Promise((resolve) => setTimeout(resolve, 350));

  const after = headingText();
  assert.notEqual(before, after, "heading text did not change after timers fired");
  assert.ok(!after.includes("0.0s"), `expected elapsed time to have advanced, got: ${after}`);

  console.log("run-timer.mjs: ok (before=%j after=%j)", before, after);
  process.exit(0); // the running setInterval would otherwise keep the process alive
}

/** Finds a node's handler id for `eventCode` whose last known button label/text matches `label`. */
function findHandler(tree, eventCode, label) {
  for (const [id, byEvent] of tree.listen) {
    if (byEvent.has(eventCode) && tree.label.get(id) === label) {
      return byEvent.get(eventCode);
    }
  }
  return undefined;
}

/** A tiny tree tracker: enough to read text/labels and find event listeners by node. */
function buildTreeTracker(app) {
  const text = new Map(); // node id -> string value (last Text op)
  const label = new Map(); // node id -> last set-prop "label"/"value" string (Prop.label = 3 in ui-api)
  const listen = new Map(); // node id -> Map<eventCode, handler>
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string") {
        label.set(op.id, op.value);
      }
    }
  };
  return { text, label, listen };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
