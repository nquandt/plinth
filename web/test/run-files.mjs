// Node test: `plinth:files` on the web host (core 1.11, docs/STORAGE.md §2,
// §3, §6 item 5). The path rules (the same table as the desktop runner's
// `files.rs` tests), the `FileSpace` provider (results, quota, call order),
// the strict check of the `files-*` bridge messages, and the fixture app on
// the real core: a round trip, the callback form, two apps that cannot read
// each other's files, path tricks, and denied calls that do not trap.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build web/test/fixtures/files-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { checkPath, FileSpace, memoryFileBackend, sharedMemoryBackend, MAX_FILE } from "../files.js";
import { checkFrameMessage, CHANNEL } from "../hub-storage.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

function pathRules() {
  const bad = [
    ["", "empty"],
    ["/etc/passwd", "absolute"],
    ["a\\b", "backslash"],
    ["C:/x", "reserved character"],
    ["c:", "reserved character"],
    ["a/../b", "dot segment"],
    ["..", "dot segment"],
    ["./a", "dot segment"],
    ["a//b", "empty segment"],
    ["a/", "empty segment"],
    ["a\u0000b", "control character"],
    ["a\nb", "control character"],
    ["a\u007f", "control character"],
    ["a\u0085", "control character"],
    ["file:stream", "reserved character"],
    ["a*", "reserved character"],
    ["a?", "reserved character"],
    ["a<b>", "reserved character"],
    ["a|b", "reserved character"],
    ['"q"', "reserved character"],
    ["name.", "trailing dot or space"],
    ["name ", "trailing dot or space"],
    ["CON", "device name"],
    ["nul.txt", "device name"],
    ["com1", "device name"],
    ["LPT9.md", "device name"],
  ];
  for (const [p, why] of bad) assert.equal(checkPath(p).error, why, JSON.stringify(p));
  assert.equal(checkPath(Array(40).fill("a").join("/")).error, "too deep");
  assert.equal(checkPath("x".repeat(256)).error, "segment too long");
  assert.equal(checkPath("abcd/".repeat(300)).error, "too long");
  assert.deepEqual(checkPath("", true).segments, []);
  assert.deepEqual(checkPath("notes/today.md").segments, ["notes", "today.md"]);
  for (const good of [".hidden", "...x", "console.md", "com10", "caf\u00e9/na\u00efve", "%2e%2e"]) {
    assert.ok(checkPath(good).segments, good);
  }
}

const okText = (r) => {
  assert.equal(r[0], true, `expected success, got ${JSON.stringify(r)}`);
  return r[2];
};
const errText = (r) => {
  assert.equal(r[0], false, `expected an error, got ${JSON.stringify(r)}`);
  return r[3];
};

async function provider() {
  const f = new FileSpace(memoryFileBackend());
  assert.equal(okText(await f.call("list", "")), "[]");
  assert.equal(errText(await f.call("read", "a.md")), "not-found");
  okText(await f.call("write", "notes/a.md", "hello \u00e9"));
  assert.equal(okText(await f.call("read", "notes/a.md")), "hello \u00e9");
  assert.equal(okText(await f.call("list", "")), '[{"kind":"dir","name":"notes","size":0}]');
  assert.equal(okText(await f.call("list", "notes")), '[{"kind":"file","name":"a.md","size":8}]');
  assert.equal(okText(await f.call("stat", "notes/a.md")), '{"kind":"file","name":"a.md","size":8}');
  assert.equal(okText(await f.call("stat", "nope")), "null");
  assert.equal(errText(await f.call("read", "notes")), "not-a-file");
  assert.equal(errText(await f.call("list", "notes/a.md")), "not-a-directory");
  assert.equal(errText(await f.call("write", "notes/a.md/x", "")), "not-a-directory");
  assert.equal(errText(await f.call("write", "notes", "")), "not-a-file");
  assert.equal(errText(await f.call("list", "missing")), "not-found");
  assert.equal(errText(await f.call("read", "../x")), "invalid-path: dot segment");
  okText(await f.call("remove", "notes/a.md"));
  assert.equal(okText(await f.call("list", "")), "[]");
  okText(await f.call("remove", "notes/a.md"));
  okText(await f.call("write", "d/e/f.txt", "1"));
  okText(await f.call("remove", "d"));
  assert.equal(okText(await f.call("list", "")), "[]");

  const q = new FileSpace(memoryFileBackend(), 10);
  okText(await q.call("write", "a", "12345"));
  okText(await q.call("write", "b", "12345"));
  assert.equal(errText(await q.call("write", "c", "1")), "quota");
  okText(await q.call("write", "a", "54321"));
  okText(await q.call("remove", "b"));
  okText(await q.call("write", "c", "1"));
  const big = new FileSpace(memoryFileBackend(), Infinity);
  assert.equal(errText(await big.call("write", "x", "a".repeat(MAX_FILE + 1))), "too-large");

  // Calls run in call order, also when nobody awaits in between.
  const o = new FileSpace(memoryFileBackend());
  const pending = [];
  for (let i = 0; i < 20; i++) {
    pending.push(o.call("write", "n", String(i)));
    pending.push(o.call("read", "n"));
  }
  const results = await Promise.all(pending);
  for (let i = 0; i < 20; i++) assert.equal(okText(results[i * 2 + 1]), String(i));
}

function bridgeMessages() {
  const m = (fields) => checkFrameMessage({ channel: CHANNEL, ...fields });
  assert.ok(m({ type: "files-read", id: 1, path: "a.md" }));
  assert.ok(m({ type: "files-write", id: 1, path: "a.md", text: "x" }));
  assert.ok(m({ type: "files-list", id: 1, path: "" }));
  assert.ok(m({ type: "files-stat", id: 1, path: "a" }));
  assert.ok(m({ type: "files-remove", id: 1, path: "a" }));
  assert.equal(m({ type: "files-read", id: "1", path: "a" }), null, "id must be a number");
  assert.equal(m({ type: "files-read", id: 1 }), null, "path is required");
  assert.equal(m({ type: "files-read", id: 1, path: 5 }), null, "path must be a string");
  assert.equal(m({ type: "files-write", id: 1, path: "a" }), null, "write needs text");
  assert.equal(m({ type: "files-write", id: 1, path: "a", text: null }), null, "text must be a string");
  assert.equal(m({ type: "files-read", id: 1, path: "a", text: "x" }), null, "only write has text");
  assert.equal(m({ type: "files-read", id: 1, path: "a".repeat(2000) }), null, "path too long");
  assert.equal(m({ type: "files-write", id: 1, path: "a", text: "x".repeat(MAX_FILE + 1) }), null, "text too long");
  assert.equal(m({ type: "files-move", id: 1, path: "a" }), null, "unknown type");
  assert.equal(checkFrameMessage({ channel: "other", type: "files-read", id: 1, path: "a" }), null);
}

/** One fixture app on the real core. */
async function startApp(coreBytes, plnt, opts = {}) {
  const { appWasm, manifestText } = await readPlnt(plnt);
  const app = new PlinthApp();
  const text = new Map();
  const label = new Map();
  const listen = new Map();
  let resultId;
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
        resultId ??= op.id;
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && op.prop === 3) {
        label.set(op.id, op.value);
      }
    }
  };
  await app.load(coreBytes, appWasm, { log: () => {}, reportError: (s) => app.errors.push(s), manifestText: opts.manifestText ?? manifestText, ...opts });
  app.errors = [];
  app.init([]);
  /** Presses the button `name` and waits for "done:…". */
  app.press = async (name) => {
    let handler;
    for (const [id, byEvent] of listen) if (label.get(id) === name) handler = byEvent.get(1);
    assert.ok(handler !== undefined, `no button ${name}`);
    text.set(resultId, "pressed");
    app.onEvent({ kind: "ui", handler, event: 1, value: null });
    const start = Date.now();
    while (!text.get(resultId).startsWith("done:")) {
      if (Date.now() - start > 5000) throw new Error(`no result for ${name}: ${text.get(resultId)}`);
      await new Promise((r) => setTimeout(r, 2));
    }
    assert.deepEqual(app.errors, [], "uncaught errors");
    return text.get(resultId).slice("done:".length);
  };
  return app;
}

async function apps() {
  const plnt = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/files-fixture/dist/files-fixture.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));

  // A round trip and the callback form, on the default (memory) space.
  const solo = await startApp(coreBytes, plnt);
  assert.equal(
    await solo.press("round"),
    'w\u00f6rld|[{"name":"notes","kind":"dir","size":0}]|[{"name":"a.md","kind":"file","size":5},{"name":"b.md","kind":"file","size":6}]|file5|null|0',
  );
  assert.equal(await solo.press("callbacks"), "null|cb|not-found|0");

  // Two apps on one page store (one Map, as one IndexedDB): a namespace each.
  const shared = new Map();
  const a = await startApp(coreBytes, plnt, { files: new FileSpace(sharedMemoryBackend(shared, "app.a")) });
  const b = await startApp(coreBytes, plnt, { files: new FileSpace(sharedMemoryBackend(shared, "app.b")) });
  assert.equal(await a.press("write"), "written");
  assert.equal(await a.press("read"), "A's secret");
  assert.equal(await b.press("read"), "err:not-found");
  assert.equal(await b.press("list"), "list:0");
  const tricks = (await b.press("tricks")).split(";").filter(Boolean);
  assert.equal(tricks.length, 13);
  tricks.forEach((t, i) => assert.ok(t.startsWith("invalid-path: "), `path trick ${i} was not refused: ${t}`));
  assert.ok(![...shared.values()].includes("x"), "the escape write stored nothing");
  assert.deepEqual([...shared.keys()], ["app.a\u0000secret.md"]);

  // Denied: refused by the user, and not declared in the manifest. The app
  // gets a rejection, and keeps running.
  const refused = await startApp(coreBytes, plnt, { refused: new Set(["files.private"]) });
  assert.equal(await refused.press("read"), "err:denied:refused");
  assert.equal(await refused.press("list"), "err:denied:refused");
  const { manifestText } = await readPlnt(plnt);
  const undeclared = await startApp(coreBytes, plnt, { manifestText: manifestText.replace('name = "files.private"', 'name = "store.kv"') });
  assert.equal(await undeclared.press("read"), "err:denied:undeclared");
  assert.equal(await undeclared.press("list"), "err:denied:undeclared");
}

async function main() {
  pathRules();
  await provider();
  bridgeMessages();
  await apps();
  console.log("run-files.mjs: ok (path rules, provider, bridge checks, round trip, callbacks, two apps isolated, 13 path tricks refused, refused and undeclared denials)");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
