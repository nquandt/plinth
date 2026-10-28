// Node test of `examples/kanban` on the web host (docs/KANBAN-FINDINGS.md):
// the board loads its first issues from `store.sql` (SQLite Wasm, in memory:
// Node has no OPFS), a `drop` event on a column moves a card (the same
// event the pointer-event drag sends in a browser), a drop on a card puts
// the dragged card before it, and a second instance on the same engine sees
// the new order. The desktop tests of the same app are
// `examples/kanban/app/board.test.tsx` and
// `crates/plinth-shoot/tests/kanban_layout.rs`.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/kanban
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
const ID = "dev.plinth.examples.kanban";

async function engine() {
  globalThis.sqlite3ApiConfig = { warn: () => {}, log: () => {}, debug: () => {}, error: () => {} };
  const wasmBinary = readFileSync(path.join(root, "web/vendor/sqlite-wasm/sqlite3.wasm"));
  return startSqlEngine(init, sqliteModuleOptions(wasmBinary, HEAP_LIMIT), {});
}

const nodes = (h) => [...h.tree.nodes.values()];
const label = (n) => n.props.get(Prop.label);
const byLabel = (h, kind, text) => nodes(h).find((n) => n.kind === kind && label(n) === text);

/** The labels of the cards under the column scroll `column`, top to bottom. */
function cards(h, column) {
  const scroll = byLabel(h, ControlKind.scroll, column);
  assert.ok(scroll, `a scroll ${column}`);
  const out = [];
  const walk = (id) => {
    const n = h.tree.node(id);
    if (!n) return;
    if (n.kind === ControlKind.pressable && /^KAN-\d+:/.test(label(n) ?? "")) out.push(label(n));
    else for (const c of n.children) walk(c);
  };
  for (const c of scroll.children) walk(c);
  return out;
}

async function until(h, what, pred, ms = 20000) {
  const startAt = Date.now();
  while (!pred()) {
    if (Date.now() - startAt > ms) {
      const seen = nodes(h).filter((n) => n.kind === ControlKind.scroll || n.kind === ControlKind.text).map((n) => `${n.kind}:${label(n) ?? n.text}`);
      assert.fail(`${what}: timed out; errors ${JSON.stringify(h.errors)}; ${JSON.stringify(seen)}`);
    }
    await new Promise((r) => setTimeout(r, 1));
  }
  assert.deepEqual(h.errors, [], `${what}: uncaught errors`);
}

async function start(build, sqlHost) {
  const app = new PlinthApp();
  const tree = new Tree();
  const errors = [];
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(build.coreBytes, build.appWasm, {
    log: () => {},
    reportError: (s) => errors.push(s),
    manifestText: build.manifestText,
    sharedMap: new Map(),
    sql: sqlHost.space(ID),
  });
  await app.init([]);
  const h = { app, tree, errors };
  await until(h, "the first issues load", () => nodes(h).some((n) => n.kind === ControlKind.text && (n.text ?? "").startsWith("10 issues")));
  return h;
}

function drop(h, node, value) {
  assert.ok(node?.listeners.get(Event.drop), "the target takes a drop");
  h.app.onEvent({ kind: "ui", handler: node.listeners.get(Event.drop), event: Event.drop, value: [value, 1, 1] });
}

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/kanban/dist/kanban.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm, manifestText } = await readPlnt(pltBytes);
  assert.ok(manifestText.includes('name = "store.sql"'), "the manifest declares store.sql");
  const build = { coreBytes, appWasm, manifestText };

  const host = localSqlHost(await engine());
  const h = await start(build, host);
  assert.deepEqual(cards(h, "In Progress issues"), ["KAN-3: Login fails on a slow network", "KAN-4: Add a dark theme"]);
  assert.equal(cards(h, "Done issues").length, 2);

  // A drop on a column: to its end.
  drop(h, byLabel(h, ControlKind.scroll, "Done issues"), "7");
  await until(h, "KAN-7 is in Done", () => cards(h, "Done issues").length === 3);
  assert.equal(cards(h, "Done issues")[2], "KAN-7: Export issues to CSV");
  assert.equal(cards(h, "Backlog issues").length, 3);

  // A drop on a card (the upper half): before it.
  const target = byLabel(h, ControlKind.pressable, "KAN-3: Login fails on a slow network");
  h.app.onEvent({ kind: "ui", handler: target.listeners.get(Event.drop), event: Event.drop, value: ["8", 1, 1] });
  await until(h, "KAN-8 is before KAN-3", () => cards(h, "In Progress issues").length === 3);
  assert.deepEqual(cards(h, "In Progress issues").map((l) => l.split(":")[0]), ["KAN-8", "KAN-3", "KAN-4"]);

  // A second instance on the same engine reads the saved order.
  await new Promise((r) => setTimeout(r, 50));
  const h2 = await start(build, host);
  await until(h2, "the saved order", () => cards(h2, "In Progress issues").length === 3);
  assert.deepEqual(cards(h2, "In Progress issues").map((l) => l.split(":")[0]), ["KAN-8", "KAN-3", "KAN-4"]);
  assert.deepEqual(cards(h2, "Done issues").map((l) => l.split(":")[0]), ["KAN-1", "KAN-2", "KAN-7"]);

  console.log("run-kanban.mjs: ok");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
