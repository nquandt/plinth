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
import { PlinthApp, readPlnt, capCoreMemory, MEMORY_LIMIT_PAGES, SRCMAP_SECTION, moduleBodies, decodeSrcMap, trapPlace } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const plnt = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/stop-fixture/dist/stop-fixture.plnt")));
const { appWasm } = await readPlnt(plnt);

// `app.wasm` with no source map (an old package): the same bytes without
// the custom section `plinth-srcmap` (docs/PRODUCTION.md L9).
function withoutSrcMap(bytes) {
  const parts = [bytes.subarray(0, 8)];
  let pos = 8;
  const leb = () => {
    let r = 0,
      shift = 0,
      b;
    do {
      b = bytes[pos++];
      r += (b & 0x7f) * 2 ** shift;
      shift += 7;
    } while (b & 0x80);
    return r;
  };
  while (pos < bytes.length) {
    const start = pos;
    const id = bytes[pos++];
    const len = leb();
    const end = pos + len;
    let keep = true;
    if (id === 0) {
      const n = leb();
      keep = new TextDecoder().decode(bytes.subarray(pos, pos + n)) !== SRCMAP_SECTION;
    }
    if (keep) parts.push(bytes.subarray(start, end));
    pos = end;
  }
  const out = new Uint8Array(parts.reduce((a, p) => a + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

async function start({ handleStop = true, app = appWasm } = {}) {
  const h = { app: new PlinthApp(), tree: new Tree(), errors: [], stops: [], logs: [] };
  h.app.onCommit = (ops) => h.tree.apply(ops);
  if (handleStop) h.app.onStop = (reason) => h.stops.push(reason);
  h.app.onError = (m) => h.errors.push(m);
  await h.app.load(coreBytes, app, { log: (s) => h.logs.push(s), reportError: () => {} });
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
  // The report names the place of the `throw` (docs/PRODUCTION.md L9).
  assert.deepEqual(h.errors, ["Uncaught Error: on purpose (app/main.tsx:19)"]);
  assert.equal(h.app.stopped, null);
  press(h, "ping");
  assert.equal(status(h), "pong 5");
}

// A failed runtime check: the core's reason and the place of the innermost
// app frame (the source map); then the app is not called.
{
  const h = await start();
  press(h, "index");
  assert.deepEqual(h.stops, ["array index out of bounds (app/main.tsx:26)"]);
  assert.equal(h.app.stopped, "array index out of bounds (app/main.tsx:26)");
  press(h, "ping");
  h.app.onEvent({ kind: "timer", timer: 1 });
  assert.equal(status(h), "start", "a stopped app is not called again");
  assert.equal(h.stops.length, 1);
}

// Endless recursion: the engine's stack limit, in plain words.
{
  const h = await start();
  press(h, "recurse");
  assert.deepEqual(h.stops, ["too many nested calls (stack overflow) (app/main.tsx:8)"]);
}

// A runaway allocation: the 64 MiB memory cap (capCoreMemory), the same
// reason as the desktop runner's memory limit.
{
  const h = await start();
  press(h, "allocate");
  assert.deepEqual(h.stops, ["out of memory (app/main.tsx:46)"]);
  const memory = Object.values(h.app._core?.instance?.exports ?? {}).find((e) => e instanceof WebAssembly.Memory);
  if (memory) assert.ok(memory.buffer.byteLength <= MEMORY_LIMIT_PAGES * 65536, "the memory stays under the cap");
}

// The cap itself: the core's memory gets a maximum, and a lower one stays.
{
  const capped = capCoreMemory(coreBytes, MEMORY_LIMIT_PAGES);
  const mod = new WebAssembly.Module(capped);
  assert.ok(WebAssembly.Module.exports(mod).some((e) => e.kind === "memory"));
  const tiny = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 4, 1, 1, 1, 2]); // memory min 1, max 2
  assert.deepEqual([...capCoreMemory(tiny, 1024).subarray(8)], [5, 4, 1, 1, 1, 2]);
  const open = new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 5, 3, 1, 0, 1]); // memory min 1, no max
  assert.deepEqual([...capCoreMemory(open, 1024).subarray(8)], [5, 5, 1, 1, 1, 128, 8]);
}

// With no `onStop` handler, the call throws (Node tests and other callers
// see the trap), and the app is still marked stopped.
{
  const h = await start({ handleStop: false });
  assert.throws(() => press(h, "index"), WebAssembly.RuntimeError);
  assert.equal(h.app.stopped, "array index out of bounds (app/main.tsx:26)");
  press(h, "ping");
  assert.equal(status(h), "start");
}

// An app module with no source map (an old package): the plain reasons, as
// before.
{
  const plain = withoutSrcMap(appWasm);
  assert.ok(plain.length < appWasm.length, "the fixture has a source map");
  assert.equal(moduleBodies(plain, SRCMAP_SECTION).custom, null);
  const h = await start({ app: plain });
  press(h, "index");
  assert.deepEqual(h.stops, ["array index out of bounds"]);
  const r = await start({ app: plain });
  press(r, "recurse");
  assert.deepEqual(r.stops, ["too many nested calls (stack overflow)"]);
}

// The map readers never throw on bad bytes, and a frame in a core body is
// not an app frame.
{
  const app = moduleBodies(appWasm, SRCMAP_SECTION);
  const map = decodeSrcMap(app.custom);
  assert.deepEqual(map.files, ["app/main.tsx"]);
  for (let cut = 0; cut < app.custom.length; cut++) assert.equal(decodeSrcMap(app.custom.subarray(0, cut)), null, `cut at ${cut}`);
  assert.equal(moduleBodies(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 10, 9, 1])), null, "a section past the end");
  const [func, f] = [...map.funcs][0];
  const at = app.bodies[func][0] + f.entries[0][0];
  const frame = `at x (wasm://wasm/1234:wasm-function[${func}]:0x${at.toString(16)})`;
  assert.equal(trapPlace(frame, app, null), `app/main.tsx:${f.entries[0][1]}`);
  const core = { bodies: [], custom: null };
  core.bodies[func] = [at, at + 1];
  assert.equal(trapPlace(frame, app, core), null, "the frame fits a core body");
}

console.log("run-stop.mjs: ok");
