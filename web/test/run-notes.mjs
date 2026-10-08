// Node test of `examples/notes` on the web host (docs/SQL.md §7 step 4,
// docs/VALIDATION.md V2): the notes are in `store.sql` (the SQLite Wasm
// engine of `web/sql.js`, in memory: Node has no OPFS), the search uses the
// FTS5 index (a prefix search on each word), search text with quotes and
// FTS5 operators never fails, the notes of an older build move from
// `store.kv` (an in-memory `Map`) into the table, and a second instance on
// the same engine sees the notes. The desktop test of the same app is
// `crates/plinth-compiler/tests/notes.rs`.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/notes
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import init from "../vendor/sqlite-wasm/sqlite3.mjs";
import { startSqlEngine, sqliteModuleOptions, localSqlHost, HEAP_LIMIT } from "../sql.js";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const ID = "dev.plinth.examples.notes";
const FIRST = ["Welcome", "Shopping list", "Idea"];

async function engine() {
  // The Wasm build warns that it cannot install its "opfs" VFS in Node.
  globalThis.sqlite3ApiConfig = { warn: () => {}, log: () => {}, debug: () => {}, error: () => {} };
  const wasmBinary = readFileSync(path.join(root, "web/vendor/sqlite-wasm/sqlite3.wasm"));
  return startSqlEngine(init, sqliteModuleOptions(wasmBinary, HEAP_LIMIT), {});
}

/** Starts one instance of the app with the kv `Map` and the SQL host. */
async function start(build, sharedMap, sqlHost) {
  const app = new PlinthApp();
  const tree = new Tree();
  const errors = [];
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(build.coreBytes, build.appWasm, {
    log: () => {},
    reportError: (s) => errors.push(s),
    manifestText: build.manifestText,
    sharedMap,
    sql: sqlHost.space(ID),
  });
  await app.init([]);
  const h = { app, tree, errors };
  await until(h, "the notes load", () => !status(h).startsWith("Loading"));
  return h;
}

const nodes = (h) => [...h.tree.nodes.values()];

function section(h, title) {
  return nodes(h).find((n) => n.kind === ControlKind.section && n.props.get(Prop.title) === title);
}

/** The text over the list: "<shown> of <all> notes", or a problem. */
function status(h) {
  const s = section(h, "All notes");
  return s ? (h.tree.node(s.children[0])?.text ?? "") : "";
}

function rows(h) {
  const list = nodes(h).find((n) => n.kind === ControlKind.list);
  return list.children.map((id) => h.tree.node(id)).filter((n) => n.kind === ControlKind.row).map((n) => n.props.get(Prop.title));
}

/** Waits until `pred()` is true: the database answers later. */
async function until(h, what, pred, ms = 20000) {
  const startAt = Date.now();
  while (!pred()) {
    if (Date.now() - startAt > ms) assert.fail(`${what}: timed out; rows ${JSON.stringify(rows(h))}, status ${JSON.stringify(status(h))}`);
    await new Promise((r) => setTimeout(r, 1));
  }
  assert.deepEqual(h.errors, [], `${what}: uncaught errors`);
  return Date.now() - startAt;
}

/** Waits until the list shows `expected` (each problem text of the app has a ':'). */
async function untilRows(h, what, expected) {
  const ms = await until(h, what, () => JSON.stringify(rows(h)) === JSON.stringify(expected) || status(h).includes(":"));
  assert.deepEqual(rows(h), expected, `${what}: status ${JSON.stringify(status(h))}`);
  assert.ok(status(h).endsWith(" notes"), `${what}: status ${JSON.stringify(status(h))}`);
  return ms;
}

function fieldIn(h, sectionTitle, label) {
  const s = section(h, sectionTitle);
  assert.ok(s, `a section ${sectionTitle}`);
  const n = s.children.map((id) => h.tree.node(id)).find((c) => c.props.get(Prop.label) === label);
  assert.ok(n, `a field ${label} in ${sectionTitle}`);
  return n;
}

function type(h, node, value) {
  h.app.onEvent({ kind: "ui", handler: node.listeners.get(Event.change), event: Event.change, value });
}

function press(h, node) {
  h.app.onEvent({ kind: "ui", handler: node.listeners.get(Event.press), event: Event.press, value: null });
}

function search(h, text) {
  type(h, fieldIn(h, "Search", "Search"), text);
}

function oldNotes(list) {
  return JSON.stringify(list.map(([id, title, body]) => ({ id, title, body })));
}

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/notes/dist/notes.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm, manifestText } = await readPlnt(pltBytes);
  assert.ok(manifestText.includes('name = "store.sql"'), "the notes manifest declares store.sql");
  assert.ok(manifestText.includes('name = "store.kv"'), "the notes manifest declares store.kv (for the move of old notes)");
  const build = { coreBytes, appWasm, manifestText };

  // A new database: the first notes, add, search, and a second instance.
  {
    const host = localSqlHost(await engine());
    const kv = new Map();
    const h = await start(build, kv, host);
    await untilRows(h, "the first notes", FIRST);
    type(h, fieldIn(h, "Add note", "Title"), "Groceries");
    type(h, fieldIn(h, "Add note", "Body"), "Bananas and milk");
    press(h, nodes(h).find((n) => n.kind === ControlKind.button && n.props.get(Prop.label) === "Add note"));
    await untilRows(h, "add", ["Groceries", ...FIRST]);

    search(h, "mil");
    await untilRows(h, "search mil", ["Groceries", "Shopping list"]);
    assert.equal(status(h), "2 of 4 notes");
    search(h, "MIL egg");
    await untilRows(h, "search two words", ["Shopping list"]);
    search(h, "zzzz");
    await untilRows(h, "no match", []);

    // Quotes and FTS5 operators are text, never syntax. The cases change
    // between a match and no match, so each wait sees its own answer.
    const cases = [
      ['"mil', ["Groceries", "Shopping list"]],
      ['"', []],
      ["mil*", ["Groceries", "Shopping list"]],
      ["*", []],
      ["-milk", ["Groceries", "Shopping list"]],
      ["-", []],
      ['"milk" "eggs"', ["Shopping list"]],
      ['a"b', []],
      ["(bananas)", ["Groceries"]],
      ["NEAR(", []],
      ["milk:", ["Groceries", "Shopping list"]],
      ["milk OR zzz", []],
      ["^mil", ["Groceries", "Shopping list"]],
      ["-- ; DROP TABLE notes", []],
    ];
    for (const [text, expected] of cases) {
      search(h, text);
      await untilRows(h, `search ${JSON.stringify(text)}`, expected);
    }
    search(h, "");
    await untilRows(h, "no search", ["Groceries", ...FIRST]);
    assert.equal(kv.size, 0, "the app writes nothing to kv");

    // A second instance on the same engine (one page) sees the note, and
    // does not add the first notes again.
    const h2 = await start(build, kv, host);
    await untilRows(h2, "the second instance", ["Groceries", ...FIRST]);
  }

  // The notes of an older build move from kv into the table, one time.
  {
    const host = localSqlHost(await engine());
    const kv = new Map([
      ["notes", oldNotes([[1, "Old one", "kept in kv"], [7, "Old two", "also old"]])],
      ["other", "stays"],
    ]);
    const h = await start(build, kv, host);
    await untilRows(h, "the moved notes", ["Old two", "Old one"]);
    assert.equal(kv.has("notes"), false, "the old key is removed");
    assert.equal(kv.get("other"), "stays");
    search(h, "kv");
    await untilRows(h, "search a moved note", ["Old one"]);
    kv.set("notes", oldNotes([[1, "Old one", "kept in kv"]]));
    const h2 = await start(build, kv, host);
    await untilRows(h2, "no second copy", ["Old two", "Old one"]);
    assert.equal(kv.has("notes"), false);
  }

  // 1,000 notes (VALIDATION V2), moved from kv in one batch.
  {
    const host = localSqlHost(await engine());
    const many = [];
    for (let i = 1; i <= 1000; i += 1) many.push([i, `Note ${i}`, `topic${i % 50} and some more words for the body of note ${i}`]);
    const kv = new Map([["notes", oldNotes(many)]]);
    const t0 = Date.now();
    const h = await start(build, kv, host);
    await until(h, "1000 notes", () => rows(h).length === 1000);
    const startMs = Date.now() - t0;
    search(h, "topic7");
    const ms7 = await until(h, "search topic7", () => rows(h).length === 20);
    assert.equal(rows(h)[0], "Note 957", "the last change first");
    search(h, "topic1");
    const ms1 = await until(h, "search topic1", () => rows(h).length === 220);
    search(h, "topic1 961");
    await untilRows(h, "two words", ["Note 961"]);
    console.log("run-notes.mjs: 1000 notes: start %d ms, search with 20 hits %d ms, with 220 hits %d ms", startMs, ms7, ms1);
  }

  console.log("run-notes.mjs: ok");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
