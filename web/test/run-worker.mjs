// Node test of the Worker split (app-worker.js, PRODUCTION R3, GAPS R1):
// `runWorker` (the Worker side) and `WorkerApp` (the document side) talk
// over a `MessageChannel`, as they do over a real `Worker` in the browser.
// It checks that the proxy keeps the behavior of `PlinthApp`: commits,
// events, uncaught errors and stop reasons, `store.kv` through the
// snapshot and the writes to the document, and frame timers driven by the
// document. The time limit (the watchdog) runs the Worker side in a real
// thread (`worker_threads`), because an endless loop must not block the
// test. run-a11y.mjs checks a real Worker in headless Edge.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/counter examples/notes examples/pong
//   target/debug/plinth.exe build web/test/fixtures/stop-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Worker as ThreadWorker } from "node:worker_threads";
import { readPlnt, mapKvStore, CALL_TIME_LIMIT_MS, TIME_LIMIT_REASON } from "../plinth-web.js";
import { runWorker, WorkerApp } from "../app-worker.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const ports = [];

/**
 * The Worker side in a real thread: `worker_threads` with the same
 * `runWorker`, and a document side with `postMessage`, `onmessage` and
 * `terminate`.
 */
function threadWorker() {
  const url = pathToFileURL(path.join(root, "web/app-worker.js")).href;
  const code = `const { parentPort } = require("node:worker_threads");
    import(${JSON.stringify(url)}).then((m) => m.runWorker({
      postMessage: (msg, transfer) => parentPort.postMessage(msg, transfer),
      set onmessage(f) { parentPort.on("message", (data) => f({ data })); },
    }));`;
  const thread = new ThreadWorker(code, { eval: true });
  const worker = {
    terminated: false,
    postMessage: (msg) => thread.postMessage(msg),
    set onmessage(f) {
      thread.on("message", (data) => f({ data }));
    },
    terminate() {
      worker.terminated = true;
      return thread.terminate();
    },
  };
  return worker;
}

async function start(plnt, opts = {}, worker = null) {
  const { appWasm, manifestText } = await readPlnt(new Uint8Array(readFileSync(path.join(root, plnt))));
  if (!worker) {
    const { port1, port2 } = new MessageChannel();
    ports.push(port1, port2);
    runWorker(port1);
    worker = port2;
  }
  const h = { app: new WorkerApp(worker), tree: new Tree(), errors: [], stops: [] };
  h.app.onCommit = (ops) => h.tree.apply(ops);
  h.app.onStop = (reason) => h.stops.push(reason);
  h.app.onError = (m) => h.errors.push(m);
  await h.app.load(core, appWasm, { log: () => {}, reportError: () => {}, manifestText, ...opts });
  await h.app.init([]);
  return h;
}

/** Waits until `pred()` is true (the Worker answers later). */
async function until(pred, what) {
  const end = Date.now() + 5000;
  while (!pred()) {
    if (Date.now() > end) assert.fail(`timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 5));
  }
}

const nodes = (h) => [...h.tree.nodes.values()];
const texts = (h) => nodes(h).map((n) => n.text ?? "").join(" | ");
function press(h, label, kind = ControlKind.button) {
  const n = nodes(h).find((n) => n.kind === kind && n.props.get(Prop.label) === label);
  assert.ok(n, `a node labelled ${label}`);
  h.app.onEvent({ kind: "ui", handler: n.listeners.get(Event.press), event: Event.press, value: null });
}

// The counter: init commits, and a press comes back as a commit.
{
  const h = await start("examples/counter/dist/counter.plnt");
  const button = nodes(h).find((n) => n.listeners.has(Event.press));
  assert.ok(button, "the counter has a press listener after init");
  const before = texts(h);
  h.app.onEvent({ kind: "ui", handler: button.listeners.get(Event.press), event: Event.press, value: null });
  await until(() => texts(h) !== before, "a press changes the text");
}

// Uncaught errors and stops: the same reasons as `PlinthApp` (run-stop.mjs).
{
  const h = await start("web/test/fixtures/stop-fixture/dist/stop-fixture.plnt");
  const status = () => nodes(h).find((n) => n.kind === ControlKind.text)?.text;
  press(h, "throw");
  await until(() => h.errors.length === 1, "the error report");
  assert.deepEqual(h.errors, ["Uncaught Error: on purpose"]);
  press(h, "ping");
  await until(() => status() === "pong 5", "the app keeps running after an error");
  assert.equal(h.app.stopped, null);
  press(h, "index");
  await until(() => h.stops.length === 1, "the stop");
  assert.deepEqual(h.stops, ["array index out of bounds"]);
  assert.equal(h.app.stopped, "array index out of bounds");
  press(h, "ping");
  await new Promise((r) => setTimeout(r, 50));
  assert.equal(status(), "pong 5", "a stopped app is not called again");
}

// store.kv: the Worker answers from the snapshot; each write comes back to
// the document's store, and a second app starts from it.
{
  const map = new Map();
  const h = await start("examples/notes/dist/notes.plnt", { kvStore: mapKvStore(map) });
  const change = (label, value) => {
    const n = nodes(h).find((n) => n.props.get(Prop.label) === label && n.listeners.has(Event.change));
    assert.ok(n, `a field labelled ${label}`);
    h.app.onEvent({ kind: "ui", handler: n.listeners.get(Event.change), event: Event.change, value });
  };
  change("Title", "Groceries");
  change("Body", "Bananas");
  press(h, "Add note");
  await until(() => map.size > 0, "the kv write reaches the document");
  const h2 = await start("examples/notes/dist/notes.plnt", { kvStore: mapKvStore(map) });
  const all = (h) => nodes(h).flatMap((n) => [n.text ?? "", ...[...n.props.values()].filter((v) => typeof v === "string")]).join(" | ");
  assert.ok(all(h2).includes("Groceries"), `the second app reads the note: ${all(h2)}`);
}

// Frame timers: the Worker asks the document for each frame.
{
  const h = await start("examples/pong/dist/pong.plnt");
  const ballLeft = () => nodes(h).find((n) => n.props.get(Prop.label) === "Ball")?.props.get(Prop.left);
  const startLeft = ballLeft();
  press(h, "Start", ControlKind.pressable);
  await until(() => ballLeft() !== startLeft, "the ball moves on frames from the document");
  press(h, "Pause", ControlKind.pressable);
  await until(() => h.app._frame === null, "the frame loop stops");
  const paused = ballLeft();
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(ballLeft(), paused, "no frames after Pause");
}

// The time limit: an endless loop in a call ends the Worker, and the app
// stops with the desktop's reason. The page side does not wait for it.
{
  assert.equal(CALL_TIME_LIMIT_MS, 2000);
  const worker = threadWorker();
  const h = await start("web/test/fixtures/stop-fixture/dist/stop-fixture.plnt", { timeLimitMs: 300 }, worker);
  const status = () => nodes(h).find((n) => n.kind === ControlKind.text)?.text;
  press(h, "ping");
  await until(() => status() === "pong 5", "a short call is not stopped");
  const t0 = Date.now();
  press(h, "loop");
  await until(() => h.stops.length === 1, "the time limit");
  const ms = Date.now() - t0;
  assert.deepEqual(h.stops, [TIME_LIMIT_REASON]);
  assert.equal(TIME_LIMIT_REASON, "the app is not responding (time limit reached)");
  assert.equal(h.app.stopped, TIME_LIMIT_REASON);
  assert.ok(worker.terminated, "the Worker is ended");
  assert.ok(ms >= 300 && ms < 2000, `stopped after ${ms} ms`);
  press(h, "ping"); // a stopped app is not called again (and does not throw)
  console.log("  the time limit stopped an endless loop after %d ms (limit 300 ms)", ms);
}

// A slow call that ends is not stopped: big-list's init (10,000 rows) with
// the default limit, in a real thread.
{
  const worker = threadWorker();
  const t0 = Date.now();
  const h = await start("examples/big-list/dist/big-list.plnt", {}, worker);
  const ms = Date.now() - t0;
  assert.equal(h.app.stopped, null, `big-list stopped: ${h.app.stopped}`);
  assert.ok(nodes(h).length > 10000, "big-list has its rows");
  assert.ok(!worker.terminated);
  await worker.terminate();
  console.log("  big-list load and init: %d ms, not stopped", ms);
}

for (const p of ports) p.close();
console.log("run-worker.mjs: ok");
