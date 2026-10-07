// Node test: a failed app on the web host's code (plinth-web.js). An
// uncaught error goes to `onError` and the app keeps running; a trap stops
// the app with a plain reason (the same words as the desktop host), and a
// stopped app is never called again. The fixture is
// web/test/fixtures/stop-fixture; crates/plinth-compiler/tests/errors.rs
// checks the same on the desktop runner.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build web/test/fixtures/stop-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const plnt = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/stop-fixture/dist/stop-fixture.plnt")));
const { appWasm } = await readPlnt(plnt);

async function start({ handleStop = true } = {}) {
  const h = { app: new PlinthApp(), tree: new Tree(), errors: [], stops: [], logs: [] };
  h.app.onCommit = (ops) => h.tree.apply(ops);
  if (handleStop) h.app.onStop = (reason) => h.stops.push(reason);
  h.app.onError = (m) => h.errors.push(m);
  await h.app.load(coreBytes, appWasm, { log: (s) => h.logs.push(s), reportError: () => {} });
  h.app.init([]);
  return h;
}

function find(h, kind, pred) {
  const stack = [h.tree.roots.get(h.tree.topScreen())];
  while (stack.length) {
    const n = h.tree.node(stack.pop());
    if (!n) continue;
    if (n.kind === kind && pred(n)) return n;
    stack.push(...n.children);
  }
  assert.fail(`no node of kind ${kind}`);
}

function press(h, label) {
  const n = find(h, ControlKind.button, (n) => n.props.get(Prop.label) === label);
  h.app.onEvent({ kind: "ui", handler: n.listeners.get(Event.press), event: Event.press, value: null });
}

const status = (h) => find(h, ControlKind.text, () => true).text;

// An uncaught error: reported, and the app keeps running.
{
  const h = await start();
  press(h, "throw");
  assert.deepEqual(h.errors, ["Uncaught Error: on purpose"]);
  assert.equal(h.app.stopped, null);
  press(h, "ping");
  assert.equal(status(h), "pong 5");
}

// A failed runtime check: the core's reason; then the app is not called.
{
  const h = await start();
  press(h, "index");
  assert.deepEqual(h.stops, ["array index out of bounds"]);
  assert.equal(h.app.stopped, "array index out of bounds");
  press(h, "ping");
  h.app.onEvent({ kind: "timer", timer: 1 });
  assert.equal(status(h), "start", "a stopped app is not called again");
  assert.equal(h.stops.length, 1);
}

// Endless recursion: the engine's stack limit, in plain words.
{
  const h = await start();
  press(h, "recurse");
  assert.deepEqual(h.stops, ["too many nested calls (stack overflow)"]);
}

// With no `onStop` handler, the call throws (Node tests and other callers
// see the trap), and the app is still marked stopped.
{
  const h = await start({ handleStop: false });
  assert.throws(() => press(h, "index"), WebAssembly.RuntimeError);
  assert.equal(h.app.stopped, "array index out of bounds");
  press(h, "ping");
  assert.equal(status(h), "start");
}

console.log("run-stop.mjs: ok");
