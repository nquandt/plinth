// Node test of `plinth:app-info`/`onUpgrade` on the web host (core 1.13,
// PRODUCTION.md D4): `data-version`/`mark-data-version` wired to a store
// kept apart from `store.kv` (`PlinthApp.load`'s `dataVersionStore`,
// `cmpVersions`/`shouldStoreDataVersion` in `plinth-web.js`), with the same
// cases as the desktop test `crates/plinth-compiler/tests/upgrade.rs`:
// first start, the same version, an upgrade, a hook that throws or
// rejects, and an async hook.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt, localStorageKvStore } from "../plinth-web.js";
import { memoryStorage, HubStore, frameDataVersionStore, checkFrameMessage, startMessage } from "../hub-storage.js";
import { runWorker, WorkerApp } from "../app-worker.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));

function text(tree) {
  for (const n of tree.nodes.values()) {
    if (n.kind === ControlKind.text) return n.text ?? "";
  }
  throw new Error("no Text found");
}

function cliPath() {
  for (const name of ["plinth.exe", "plinth"]) {
    const p = path.join(root, "target/debug", name);
    if (existsSync(p)) return p;
  }
  throw new Error("build the CLI first: cargo build -p plinth-cli");
}

/** An app whose `onUpgrade` handler shows `fromVersion` (or "none") and a
 * run count in a `Text`; `throws`/`rejects` select how the handler fails
 * (the same two app shapes as the desktop test's `SYNC_APP`/`ASYNC_APP`). */
function appSource({ async: isAsync, throws, rejects }) {
  const body = isAsync
    ? `onUpgrade(async (fromVersion) => {
  runs.update((n) => n + 1);
  from.set(fromVersion);
  if (${throws}) {
    throw new Error("upgrade failed");
  }
  await new Promise<void>((resolve, reject) => {
    setTimeout(() => {
      if (${rejects}) {
        reject(new Error("upgrade rejected"));
        return;
      }
      from.set(fromVersion + " (done)");
      resolve();
    }, 10);
  });
});`
    : `onUpgrade((fromVersion) => {
  runs.update((n) => n + 1);
  from.set(fromVersion);
  if (${throws}) {
    throw new Error("upgrade failed");
  }
});`;
  return `
import { app, signal, Screen, Text, onUpgrade } from "plinth:ui";
import { setTimeout } from "plinth:time";

const from = signal("none");
const runs = signal(0);

${body}

function Home() {
  return (
    <Screen title="Home">
      <Text>{\`\${from()} \${runs()}\`}</Text>
    </Screen>
  );
}
export default app({ screens: { home: { title: "Home", component: Home } } });
`;
}

/** Builds a `.plnt` with `id`/`version` in its manifest and `source` as `app/main.tsx`. */
function build(id, version, source) {
  const dir = path.join(os.tmpdir(), `plinth-run-upgrade-${id}-${process.pid}-${Math.random().toString(36).slice(2)}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(path.join(dir, "app"), { recursive: true });
  writeFileSync(path.join(dir, "plinth.toml"), `id = "${id}"\nname = "Upgrade"\nversion = "${version}"\npublisher = "plinth"\n`);
  writeFileSync(path.join(dir, "app/main.tsx"), source);
  try {
    execFileSync(cliPath(), ["build", dir], { stdio: ["ignore", "ignore", "pipe"], encoding: "utf-8" });
  } catch (err) {
    throw new Error(`plinth build failed:\n${err.stderr ?? err}`);
  }
  const file = readdirSync(path.join(dir, "dist")).find((f) => f.endsWith(".plnt"));
  const plnt = new Uint8Array(readFileSync(path.join(dir, "dist", file)));
  rmSync(dir, { recursive: true, force: true });
  return plnt;
}

/** Starts the app at `version` against the shared `storage` (persists the
 * data version across calls, as a real browser origin does). */
async function start(storage, plnt) {
  const { appWasm, manifestText } = await readPlnt(plnt);
  const app = new PlinthApp();
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  const errors = [];
  await app.load(core, appWasm, { manifestText, storage, log: () => {}, reportError: (s) => errors.push(s) });
  app.init();
  return { app, tree, errors };
}

const APP_ID = "dev.plinth.test.upgrade-fixture";

// -- cmpVersions / shouldStoreDataVersion: the same cases as the desktop's
// `app_info::cmp_versions`/`should_store` (`crates/plinth-runner-wasmtime/src/app_info.rs`).
{
  const { cmpVersions, shouldStoreDataVersion } = await import("../plinth-web.js");
  assert.equal(cmpVersions("1.0.0", "1.0.0"), 0);
  assert.equal(cmpVersions("1.2.0", "1.10.0"), -1);
  assert.equal(cmpVersions("2.0.0", "1.9.9"), 1);
  assert.equal(cmpVersions("1.0.10", "1.0.2"), 1);
  assert.equal(cmpVersions("1.0.0-rc.1", "1.0.0"), -1);
  assert.equal(cmpVersions("1.0.0", "1.0.0-rc.1"), 1);
  assert.equal(cmpVersions("1.0.0-rc.1", "1.0.0-rc.2"), 0);
  assert.equal(cmpVersions("1.2", "1.2.0"), 0);
  assert.equal(cmpVersions("x.y.z", "0.0.0"), 0);
  assert.equal(shouldStoreDataVersion(null, "1.0.0"), true);
  assert.equal(shouldStoreDataVersion("1.0.0", "1.1.0"), true);
  assert.equal(shouldStoreDataVersion("1.0.0", "1.0.0"), true);
  assert.equal(shouldStoreDataVersion("2.0.0", "1.5.0"), false);
}

// -- First start: no hook call, the version is stored ---------------------
{
  const storage = memoryStorage();
  const plnt = build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false }));
  const { tree } = await start(storage, plnt);
  assert.equal(text(tree), "none 0");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0");
}

// -- The same version again: no hook call ----------------------------------
{
  const storage = memoryStorage();
  const plnt = build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false }));
  await start(storage, plnt);
  const { tree } = await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  assert.equal(text(tree), "none 0");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0");
}

// -- An upgrade: the hook runs with the old version, then it is marked -----
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  const { tree } = await start(storage, build(APP_ID, "1.1.0", appSource({ async: false, throws: false, rejects: false })));
  assert.equal(text(tree), "1.0.0 1");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.1.0");
}

// -- A hook that throws: not marked, so it runs again next start -----------
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  const { tree, errors } = await start(storage, build(APP_ID, "1.1.0", appSource({ async: false, throws: true, rejects: false })));
  assert.equal(text(tree), "1.0.0 1");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0", "the throw must not store the new version");
  assert.equal(errors.length, 1, "the throw is reported as uncaught");

  const { tree: tree2 } = await start(storage, build(APP_ID, "1.1.0", appSource({ async: false, throws: true, rejects: false })));
  assert.equal(text(tree2), "1.0.0 1");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0");
}

// -- An async hook: marked only after it resolves ---------------------------
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  const { tree, app } = await start(storage, build(APP_ID, "1.1.0", appSource({ async: true, throws: false, rejects: false })));
  assert.equal(text(tree), "1.0.0 1");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0", "the version is not bumped before the timer fires");

  const deadline = Date.now() + 5000;
  while (text(tree) !== "1.0.0 (done) 1" && Date.now() < deadline) await new Promise((r) => setTimeout(r, 5));
  assert.equal(text(tree), "1.0.0 (done) 1");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.1.0");
  assert.equal(app.stopped, null);
}

// -- An async hook that rejects: not marked, so it runs again next start ----
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  const { tree, app, errors } = await start(storage, build(APP_ID, "1.1.0", appSource({ async: true, throws: false, rejects: true })));
  assert.equal(text(tree), "1.0.0 1");

  const deadline = Date.now() + 5000;
  while (errors.length === 0 && Date.now() < deadline) await new Promise((r) => setTimeout(r, 5));
  assert.equal(errors.length, 1, "the rejection is reported as uncaught");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "1.0.0", "the rejection must not store the new version");
  assert.equal(app.stopped, null);
}

// -- A downgrade: no hook call, keeps the higher version ---------------------
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "2.0.0", appSource({ async: false, throws: false, rejects: false })));
  const { tree } = await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  assert.equal(text(tree), "none 0");
  assert.equal(storage.getItem(`plinth-data-version:${APP_ID}`), "2.0.0");
}

// -- The data version store never collides with `store.kv` keys -------------
{
  const storage = memoryStorage();
  await start(storage, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  // The data version lives at a reserved prefix ("plinth-data-version:"),
  // never at the app's own kv prefix ("plinth:<id>:").
  assert.equal(storage.getItem(`plinth:${APP_ID}:data-version`), null);
}

// -- Through the Worker (`WorkerApp`): the data version survives a reload --
// `WorkerApp.load`'s `opts.dataVersionStore` carries the stored value into
// the Worker's `load` message and writes each change back (app-worker.js),
// the same round trip `opts.kvStore` does for `store.kv`. Two `WorkerApp`
// instances over the SAME `dataVersionStore` is a reload of the page: the
// second one must see what the first one stored.
async function startWorker(dataVersionStore, plnt) {
  const { appWasm, manifestText } = await readPlnt(plnt);
  const { port1, port2 } = new MessageChannel();
  runWorker(port1);
  const app = new WorkerApp(port2);
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  const errors = [];
  app.onError = (s) => errors.push(s);
  await app.load(core, appWasm, { manifestText, dataVersionStore, log: () => {}, reportError: () => {} });
  await app.init([]);
  return { app, tree, errors, close: () => (port1.close(), port2.close()) };
}

{
  const dataVersionStore = localStorageKvStore(memoryStorage(), "plinth-data-version:");

  let h = await startWorker(dataVersionStore, build(APP_ID, "1.0.0", appSource({ async: false, throws: false, rejects: false })));
  assert.equal(text(h.tree), "none 0");
  // `data-version-set` is a fire-and-forget message; give it a turn to land.
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(dataVersionStore.get(APP_ID), "1.0.0");
  h.close();

  // The "reload": a second `WorkerApp`, the same store, a newer version.
  h = await startWorker(dataVersionStore, build(APP_ID, "1.1.0", appSource({ async: false, throws: false, rejects: false })));
  assert.equal(text(h.tree), "1.0.0 1", "the Worker saw the pending upgrade after the reload");
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(dataVersionStore.get(APP_ID), "1.1.0");
  h.close();
}

// -- Through the web App Hub's frame storage (hub-storage.js), no DOM -------
// `startMessage` sends the hub page's stored data version to the frame;
// `frameDataVersionStore` answers it at once and sends each write back as
// a `data-version-set` message, the same idea as `frameKvStore` for kv;
// the page applies it with `HubStore.setDataVersion` (never under the kv
// prefix, `HubStore.kvClear` removes it too).
{
  const appId = "com.example.hub-upgrade";
  const store = new HubStore(memoryStorage());
  assert.equal(store.dataVersion(appId), null);

  // The page's own `onUpgrade`-equivalent bookkeeping: first start.
  const start1 = startMessage(store, appId, { pkg: new ArrayBuffer(0), core: new ArrayBuffer(0), refused: new Set() });
  assert.equal(start1.dataVersion, null);
  const sent1 = [];
  const frameStore1 = frameDataVersionStore(start1.dataVersion, (m) => sent1.push(m));
  assert.equal(frameStore1.get(), null);
  frameStore1.set(appId, "1.0.0"); // `finishStart` on the frame side
  assert.deepEqual(sent1.map((m) => [m.type, m.version]), [["data-version-set", "1.0.0"]]);
  for (const m of sent1) {
    const checked = checkFrameMessage(m);
    assert.ok(checked, `a valid frame message: ${JSON.stringify(m)}`);
    assert.equal(checked.type, "data-version-set");
    store.setDataVersion(appId, checked.version);
  }
  assert.equal(store.dataVersion(appId), "1.0.0");

  // A second "start" (an upgrade to 1.1.0): the frame gets 1.0.0 back.
  const start2 = startMessage(store, appId, { pkg: new ArrayBuffer(0), core: new ArrayBuffer(0), refused: new Set() });
  assert.equal(start2.dataVersion, "1.0.0");

  // `kvClear` (an uninstall) removes the data version along with the kv data.
  store.kvSet(appId, "k", "v");
  store.kvClear(appId);
  assert.equal(store.dataVersion(appId), null);
  assert.deepEqual(store.kvSnapshot(appId), {});
}

console.log("run-upgrade: all checks passed");
