// Node test (docs/web-hub.md §4): the storage protocol between the web App
// Hub page and a sandboxed app frame (`hub-storage.js`), with no DOM.
//
// 1. The parent store keeps each app's kv data under its own namespace;
//    the snapshot of one app has no key of a different app.
// 2. The frame kv store answers at once from the snapshot, sends each
//    write as a message, and applies the quota; the parent applies the
//    messages and gets the same data.
// 3. The parent refuses messages that are not valid.
// 4. Notes runs with the frame kv store (as in the frame): a note goes
//    through the messages into the parent store, and a second run starts
//    from the new snapshot and shows the note. With `store.kv` refused,
//    notes runs, and its kv calls answer "denied" (no trap, nothing stored).
//
// Build first: target/debug/plinth build examples/notes; plinth core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt, netDenied } from "../plinth-web.js";
import {
  HubStore,
  memoryStorage,
  frameKvStore,
  checkFrameMessage,
  applyKvMessage,
  startMessage,
  CHANNEL,
  KV_QUOTA,
  MAX_KEY,
} from "../hub-storage.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

function protocolChecks() {
  const backing = new Map();
  const store = new HubStore(memoryStorage(backing), memoryStorage());

  // Namespaces: two apps, the same key.
  assert.equal(store.kvSet("com.example.a", "k", "from a"), true);
  assert.equal(store.kvSet("com.example.b", "k", "from b"), true);
  assert.equal(store.kvSet("com.example.ab", "x", "1"), true, "an id that starts like another id");
  assert.deepEqual(store.kvSnapshot("com.example.a"), { k: "from a" });
  assert.deepEqual(store.kvSnapshot("COM.Example.B"), { k: "from b" }, "app ids are compared in lower case");
  assert.deepEqual(store.kvSnapshot("com.example.ab"), { x: "1" });
  assert.equal(store.kvUsage("com.example.a"), "k".length + "from a".length);

  // Quota on the parent side.
  assert.equal(store.kvSet("com.example.a", "big", "x".repeat(10), 12), false, "over the quota");
  assert.equal(store.kvSnapshot("com.example.a").big, undefined);
  assert.equal(store.kvSet("com.example.a", "k", "x".repeat(10), 12), true, "a replaced value counts once");

  // The frame side: answers at once, sends each write, keeps the quota.
  const sent = [];
  const kv = frameKvStore(store.kvSnapshot("com.example.b"), 40, (m) => sent.push(m));
  assert.equal(kv.get("k"), "from b");
  assert.equal(kv.get("none"), null);
  assert.equal(kv.set("n", "1"), true);
  assert.equal(kv.get("n"), "1");
  kv.delete("k");
  kv.delete("missing"); // no message for a missing key
  assert.deepEqual(kv.keys(), ["n"]);
  assert.equal(kv.set("huge", "y".repeat(100)), false, "the frame refuses a write over the quota");
  assert.equal(kv.set("k".repeat(MAX_KEY + 1), "v"), false, "a key that is too long");
  assert.deepEqual(
    sent.map((m) => [m.type, m.key]),
    [
      ["kv-set", "n"],
      ["kv-delete", "k"],
    ],
  );
  for (const m of sent) {
    const checked = checkFrameMessage(m);
    assert.ok(checked, `a frame message is valid: ${JSON.stringify(m)}`);
    assert.equal(applyKvMessage(store, "com.example.b", checked), true);
  }
  assert.deepEqual(store.kvSnapshot("com.example.b"), { n: "1" }, "the parent has the same data as the frame");
  assert.deepEqual(store.kvSnapshot("com.example.a"), { k: "x".repeat(10) }, "other apps do not change");

  // Not valid: wrong channel, unknown type, wrong field types, bad dialog kind.
  for (const bad of [
    null,
    "kv-set",
    { type: "kv-set", key: "a", value: "b" },
    { channel: CHANNEL, type: "kv-clear" },
    { channel: CHANNEL, type: "kv-set", key: 1, value: "b" },
    { channel: CHANNEL, type: "kv-set", key: "a" },
    { channel: CHANNEL, type: "kv-set", key: "k".repeat(MAX_KEY + 1), value: "b" },
    { channel: CHANNEL, type: "dialog", id: 1, kind: "open", message: "x" },
    { channel: CHANNEL, type: "clipboard-read", id: "1" },
  ]) {
    assert.equal(checkFrameMessage(bad), null, `refused: ${JSON.stringify(bad)?.slice(0, 80)}`);
  }
  assert.equal(applyKvMessage(store, "com.example.b", { type: "clipboard-write", text: "x" }), false);

  // net-fetch and size (the parent makes the request after its own check).
  const fetchMsg = { channel: CHANNEL, type: "net-fetch", id: 3, url: "https://api.example.com/x", method: "GET", headers: [["accept", "text/plain"]], body: null };
  assert.ok(checkFrameMessage(fetchMsg));
  assert.equal(checkFrameMessage({ ...fetchMsg, headers: [["a"]] }), null);
  assert.equal(checkFrameMessage({ ...fetchMsg, body: 5 }), null);
  assert.ok(checkFrameMessage({ channel: CHANNEL, type: "size", height: 480 }));
  assert.equal(checkFrameMessage({ channel: CHANNEL, type: "size", height: -1 }), null);
  const declared = new Set(["net:api.example.com", "net.local"]);
  assert.equal(netDenied("https://api.example.com/x", declared), null);
  assert.equal(netDenied("https://other.example.com/x", declared), "denied:undeclared");
  assert.equal(netDenied("https://api.example.com/x", declared, new Set(["net:api.example.com"])), "denied:refused");
  assert.equal(netDenied("http://192.168.1.4/", declared), null);
  assert.equal(netDenied("http://127.0.0.1/", new Set(["net:*"])), "denied:undeclared", "net:* does not cover private hosts");
  assert.equal(netDenied("not a url", declared), "denied:unsupported");

  // The start message has the data of one app only.
  const start = startMessage(store, "com.example.a", { pkg: new ArrayBuffer(1), core: new ArrayBuffer(1), refused: new Set(["clipboard.read"]) });
  assert.deepEqual(start.kv, { k: "x".repeat(10) });
  assert.equal(start.quota, KV_QUOTA);
  assert.deepEqual(start.refused, ["clipboard.read"]);
  assert.equal(start.channel, CHANNEL);

  // Grant records, session records, the app list, removal.
  store.saveGrants("com.example.a", { id: "com.example.a", name: "A", grants: {} }, { grants: { "net:*": { allowed: true } } });
  assert.equal(store.grants("com.example.a").name, "A");
  assert.equal(store.sessionGrants("com.example.a").grants["net:*"].allowed, true);
  assert.deepEqual(
    store.knownApps().map((a) => [a.id, a.name]),
    [
      ["com.example.a", "A"],
      ["com.example.ab", "com.example.ab"],
      ["com.example.b", "com.example.b"],
    ],
  );
  store.kvClear("com.example.a");
  assert.deepEqual(store.kvSnapshot("com.example.a"), {});
  assert.deepEqual(store.kvSnapshot("com.example.ab"), { x: "1" }, "clear removes only one app");
  store.forgetGrants("com.example.a");
  assert.equal(store.grants("com.example.a"), null);
  assert.equal(store.sessionGrants("com.example.a"), null);
  assert.ok(!store.knownApps().some((a) => a.id === "com.example.a"));
}

/** Runs notes as the frame does; returns its texts and a function that adds a note. */
async function runNotes(coreBytes, appWasm, manifestText, kvStore, refused = new Set()) {
  const app = new PlinthApp();
  const texts = [];
  const listen = new Map();
  const label = new Map();
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") texts.push(String(op.value));
      else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && (op.prop === 3 || op.prop === 1)) {
        label.set(op.id, op.value);
        if (op.prop === 1) texts.push(op.value);
      }
    }
  };
  const errors = [];
  await app.load(coreBytes, appWasm, { log: () => {}, reportError: (s) => errors.push(s), manifestText, kvStore, refused });
  app.init([]);
  const handler = (event, name) => {
    for (const [id, byEvent] of listen) if (byEvent.has(event) && label.get(id) === name) return byEvent.get(event);
    throw new Error(`no handler for ${name}`);
  };
  const addNote = (title, body) => {
    app.onEvent({ kind: "ui", handler: handler(2, "Title"), event: 2, value: title });
    app.onEvent({ kind: "ui", handler: handler(2, "Body"), event: 2, value: body });
    app.onEvent({ kind: "ui", handler: handler(1, "Add note"), event: 1, value: null });
  };
  return { texts, addNote, errors };
}

async function notesThroughTheProtocol() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/notes/dist/notes.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm, manifestText } = await readPlnt(pltBytes);
  const id = "dev.plinth.examples.notes";
  const store = new HubStore(memoryStorage(), memoryStorage());
  store.kvSet("dev.plinth.examples.budget", "secret", "of budget");

  // The page sends the start message; the frame makes its kv store from it.
  const toParent = (m) => {
    // structuredClone: postMessage copies the message.
    const msg = checkFrameMessage(structuredClone(m));
    assert.ok(msg);
    applyKvMessage(store, id, msg);
  };
  const start1 = startMessage(store, id, { pkg: null, core: null, refused: [] });
  assert.deepEqual(start1.kv, {}, "notes gets no data of budget");
  const kv1 = frameKvStore(start1.kv, start1.quota, toParent);
  const run1 = await runNotes(coreBytes, appWasm, manifestText, kv1);
  run1.addNote("Groceries", "Bananas");
  assert.ok(run1.texts.some((t) => t.includes("Groceries")));
  assert.ok(Object.keys(store.kvSnapshot(id)).length > 0, "the write reached the parent store");
  assert.deepEqual(store.kvSnapshot("dev.plinth.examples.budget"), { secret: "of budget" });

  // A new run starts from the new snapshot.
  const start2 = startMessage(store, id, { pkg: null, core: null, refused: [] });
  assert.ok(!("secret" in start2.kv));
  const run2 = await runNotes(coreBytes, appWasm, manifestText, frameKvStore(start2.kv, start2.quota, toParent));
  assert.ok(run2.texts.some((t) => t.includes("Groceries")), `the second run shows the note: ${run2.texts.join(" | ")}`);

  // store.kv refused: notes runs, nothing is stored, nothing traps.
  const before = JSON.stringify(store.kvSnapshot(id));
  const run3 = await runNotes(coreBytes, appWasm, manifestText, frameKvStore({}, KV_QUOTA, toParent), new Set(["store.kv"]));
  assert.ok(!run3.texts.some((t) => t.includes("Groceries")), "a refused store reads nothing");
  run3.addNote("Hidden", "No store");
  assert.ok(run3.texts.some((t) => t.includes("Hidden")), "the app keeps running with the store refused");
  assert.equal(JSON.stringify(store.kvSnapshot(id)), before, "a refused store writes nothing");

  // The quota: a write over it answers "denied" in the app (no trap), and the parent keeps the old data.
  const run4 = await runNotes(coreBytes, appWasm, manifestText, frameKvStore(store.kvSnapshot(id), 10, toParent));
  run4.addNote("Too big", "x".repeat(50));
  assert.equal(JSON.stringify(store.kvSnapshot(id)), before, "a write over the quota does not reach the parent");
}

async function main() {
  protocolChecks();
  await notesThroughTheProtocol();
  console.log("run-hub-storage.mjs: ok (namespaces, snapshot and writes, quota, message checks, notes through the protocol)");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
