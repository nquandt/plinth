// Node test of `plinth:locale` on the web host (core 1.13, docs/I18N.md §3.1,
// §3.5, §4 step 1; docs/host-apis.md "plinth:locale"): the BCP 47 tag and
// the direction (`normalizeLocale`, `isRtlLocale`, `pageLocale`, the same
// cases as `crates/plinth-runner-wasmtime/src/locale.rs`), the app on
// `PlinthApp` and through the Worker (`WorkerApp`) with a fixed locale, the
// locale event (`setLocale`: the text, a computed value and an effect run
// again), `lang` and `dir` on the root element (`applyLocale`), and the
// parent page (`AppFrame`): the locale in the `start` message, and a
// `languagechange` of the page that goes to the frame.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build web/test/fixtures/locale-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt, normalizeLocale, isRtlLocale, pageLocale, FALLBACK_LOCALE } from "../plinth-web.js";
import { runWorker, WorkerApp } from "../app-worker.js";
import { Tree, applyLocale } from "../dom-renderer.js";
import { encodeEvent, EventCode, LifecycleKind } from "../protocol.js";
import { ControlKind } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const { appWasm, manifestText } = await readPlnt(new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/locale-fixture/dist/locale-fixture.plnt"))));

// -- The tag and the direction (the same cases as the desktop host) -----------
{
  const norm = [
    ["en-US", "en-US"],
    ["en_us.UTF-8", "en-US"],
    ["de_DE@euro", "de-DE"],
    ["zh_hans_cn", "zh-Hans-CN"],
    ["ES-419", "es-419"],
    ["fr", "fr"],
    ["", "en-US"],
    ["C", "en-US"],
    ["POSIX", "en-US"],
    ["C.UTF-8", "en-US"],
  ];
  for (const [name, tag] of norm) assert.equal(normalizeLocale(name), tag, name);
  assert.equal(normalizeLocale(undefined), FALLBACK_LOCALE);
  for (const tag of ["ar", "ar-EG", "he", "he-IL", "fa-IR", "ur-PK", "ps", "sd", "ug", "yi", "dv", "ckb", "az-Arab", "pa-Arab-PK"]) {
    assert.equal(isRtlLocale(tag), true, `${tag} is right to left`);
  }
  for (const tag of ["en-US", "fr", "zh-Hans-CN", "ku", "az-Latn", "sr-Cyrl", "ja", "hi-IN", "uz-Latn-UZ"]) {
    assert.equal(isRtlLocale(tag), false, `${tag} is left to right`);
  }
  assert.equal(pageLocale({ languages: ["he-IL", "en-US"], language: "en-US" }), "he-IL");
  assert.equal(pageLocale({ languages: [], language: "fr_FR" }), "fr-FR");
  assert.equal(pageLocale({}), "en-US");
  assert.equal(pageLocale(undefined), "en-US");
}

// -- The event on the wire -----------------------------------------------------
{
  const bytes = encodeEvent({ kind: "lifecycle", lifecycleKind: LifecycleKind.LOCALE });
  assert.deepEqual([...bytes], [EventCode.LIFECYCLE, 4]);
}

const texts = (tree) =>
  [...tree.nodes.values()]
    .filter((n) => n.kind === ControlKind.text)
    .sort((a, b) => a.id - b.id)
    .map((n) => n.text ?? "");

// -- PlinthApp: the values for each locale --------------------------------------
async function start(locale) {
  const logs = [];
  const app = new PlinthApp();
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(core, appWasm, { manifestText, locale, log: (s) => logs.push(s), reportError: (s) => assert.fail(s) });
  app.init();
  return { app, tree, logs };
}

for (const [tag, dir, side] of [
  ["en-US", "ltr", "left"],
  ["ar-EG", "rtl", "right"],
  ["he", "rtl", "right"],
  ["zh-Hans-CN", "ltr", "left"],
  ["fr", "ltr", "left"],
]) {
  const { tree, logs } = await start(tag);
  assert.deepEqual(texts(tree), [`${tag} ${dir}`, side, `first ${dir}`, "runs 1"], tag);
  assert.deepEqual(logs, [`locale ${tag}`]);
}

// -- PlinthApp: a locale change runs the UI again --------------------------------
{
  const { app, tree, logs } = await start("en-US");
  logs.length = 0;
  app.setLocale("ar-EG");
  assert.deepEqual(texts(tree), ["ar-EG rtl", "right", "first ltr", "runs 2"]);
  assert.deepEqual(logs, ["locale ar-EG"]);
  app.setLocale("ar_EG"); // the same locale: no event
  assert.deepEqual(texts(tree), ["ar-EG rtl", "right", "first ltr", "runs 2"]);
  app.setLocale("fr");
  assert.deepEqual(texts(tree), ["fr ltr", "left", "first ltr", "runs 3"]);
  assert.equal(app.stopped, null);
}

// -- Without a locale, the app takes the locale of its global scope ----------
{
  const { tree } = await start(undefined);
  assert.equal(texts(tree)[0], `${pageLocale()} ${isRtlLocale(pageLocale()) ? "rtl" : "ltr"}`);
}

// -- Through the Worker: the locale in `load`, and `setLocale` -------------------
{
  const { port1, port2 } = new MessageChannel();
  runWorker(port1);
  const app = new WorkerApp(port2);
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(core, appWasm, { manifestText, locale: "he", log: () => {}, reportError: () => {} });
  await app.init([]);
  assert.deepEqual(texts(tree), ["he rtl", "right", "first rtl", "runs 1"]);
  app.setLocale("en-US");
  const end = Date.now() + 5000;
  while (texts(tree)[0] !== "en-US ltr" && Date.now() < end) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(texts(tree), ["en-US ltr", "left", "first rtl", "runs 2"]);
  port1.close();
  port2.close();
}

// -- `lang` and `dir` on the root element ----------------------------------------
{
  const attrs = {};
  const element = { setAttribute: (k, v) => (attrs[k] = v) };
  assert.equal(applyLocale(element, "ar_EG"), "ar-EG");
  assert.deepEqual(attrs, { lang: "ar-EG", dir: "rtl" });
  applyLocale(element, "zh-Hans-CN");
  assert.deepEqual(attrs, { lang: "zh-Hans-CN", dir: "ltr" });
}

// -- The parent page (AppFrame): `start` has the locale; `languagechange` --------
{
  const listeners = new Map();
  const posted = [];
  const contentWindow = { postMessage: (msg) => posted.push(msg) };
  const nav = { languages: ["de-CH"], language: "de-CH" };
  Object.defineProperty(globalThis, "navigator", { value: nav, configurable: true });
  globalThis.window = {
    addEventListener: (type, f) => listeners.set(type, f),
    removeEventListener: (type) => listeners.delete(type),
  };
  globalThis.document = {
    createElement: () => ({ setAttribute() {}, append() {}, remove() {}, dataset: {}, contentWindow }),
  };
  const { AppFrame } = await import("../frame-host.js");
  const { CHANNEL, HubStore, memoryStorage } = await import("../hub-storage.js");
  const store = new HubStore(memoryStorage());
  const frame = new AppFrame({ appId: "dev.plinth.test.locale-fixture", pkg: new Uint8Array(1), core: new Uint8Array(1), store, declared: [] });
  frame.handle({ source: contentWindow, data: { channel: CHANNEL, type: "ready" } });
  const start = posted.find((m) => m.type === "start");
  assert.equal(start.locale, "de-CH");
  nav.languages = ["ar-EG"];
  listeners.get("languagechange")();
  assert.deepEqual(posted.at(-1), { channel: CHANNEL, type: "locale", tag: "ar-EG" });
  const count = posted.length;
  listeners.get("languagechange")(); // the same locale: no message
  assert.equal(posted.length, count);
  frame.setLocale("fa");
  assert.deepEqual(posted.at(-1), { channel: CHANNEL, type: "locale", tag: "fa" });
  // A fixed locale (`opts.locale`) does not follow the page.
  const fixed = new AppFrame({ appId: "x", pkg: new Uint8Array(1), core: new Uint8Array(1), store, declared: [], locale: "he" });
  assert.equal(fixed.locale, "he");
  frame.destroy();
  assert.equal(listeners.has("languagechange"), false);
}

console.log("run-locale: all checks passed");
