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
// Step 2 (docs/I18N.md §3.7): the formatting functions with the browser's
// `Intl` (Node has full ICU), with the SAME cases as the desktop test
// `crates/plinth-compiler/tests/locale_format.rs` (the shared file
// `crates/plinth-compiler/tests/locale_cases.txt`): one app, built by the
// CLI, in each locale, on the real core. The time zone is UTC.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build web/test/fixtures/locale-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import {
  PlinthApp,
  readPlnt,
  normalizeLocale,
  isRtlLocale,
  pageLocale,
  FALLBACK_LOCALE,
  formatNumberIntl,
  formatDateIntl,
  pluralCategoryIntl,
  formatListIntl,
  compareIntl,
} from "../plinth-web.js";
import { runWorker, WorkerApp } from "../app-worker.js";
import { Tree, applyLocale } from "../dom-renderer.js";
import { encodeEvent, EventCode, LifecycleKind } from "../protocol.js";
import { ControlKind } from "../ui-api.js";

// The formatting cases use UTC, as the desktop test (`set_fake_timezone_offset(0)`).
process.env.TZ = "UTC";

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

// -- Formatting (step 2): the shared cases ------------------------------------

const LOCALES = ["en-US", "de-DE", "fr-FR", "ar-EG", "ja-JP", "hi-IN", "pl-PL", "ru-RU"];

/** `\u{hex}` and `\\` in a text of the case file. */
function unescapeText(s) {
  return s.replace(/\\(\\|u\{([0-9a-fA-F]+)\})/g, (_, all, hex) => (hex === undefined ? "\\" : String.fromCodePoint(parseInt(hex, 16))));
}

/** Reads `locale_cases.txt`: `[{ name, expr, web: { locale: text } }]`, with the checks of the Rust reader. */
function readLocaleCases(text) {
  const cases = [];
  const hostLines = [];
  const known = new Set();
  let wantExpr = false;
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const head = line.match(/^=== case (\S+)$/);
    if (head) {
      cases.push({ name: head[1], expr: "", web: {} });
      wantExpr = true;
      continue;
    }
    if (line.trim() === "" || line.startsWith("#")) continue;
    const c = cases.at(-1);
    if (c === undefined) throw new Error(`locale_cases.txt: a line outside a case: ${line}`);
    if (wantExpr) {
      c.expr = line;
      wantExpr = false;
      continue;
    }
    const k = line.match(/^known (\S+): (.+)$/);
    if (k) {
      known.add(`${c.name} ${k[1]}`);
      continue;
    }
    const space = line.indexOf(" ");
    const headPart = space < 0 ? line : line.slice(0, space);
    const value = space < 0 ? "" : line.slice(space + 1);
    const [loc, host] = headPart.split("@");
    assert.ok(LOCALES.includes(loc), `case ${c.name}: an unknown locale ${loc}`);
    if (host !== undefined) {
      assert.ok(host === "desktop" || host === "web", `case ${c.name}: an unknown host ${host}`);
      hostLines.push(`${c.name} ${loc}`);
    }
    if (host !== "desktop") {
      assert.equal(c.web[loc], undefined, `case ${c.name}: two lines for ${loc}`);
      c.web[loc] = unescapeText(value);
    }
  }
  for (const h of hostLines) assert.ok(known.has(h), `case ${h}: host lines but no "known" line`);
  for (const c of cases) for (const loc of LOCALES) assert.ok(loc in c.web, `case ${c.name}: no text for ${loc}`);
  return cases;
}

/** Finds the CLI binary. */
function cliPath() {
  for (const name of ["plinth.exe", "plinth"]) {
    const p = path.join(root, "target/debug", name);
    if (existsSync(p)) return p;
  }
  throw new Error("build the CLI first: cargo build -p plinth-cli");
}

/** Builds the app of the cases (one `Text` for each case) with the CLI. */
function buildFormatApp(cases) {
  const dir = path.join(os.tmpdir(), `plinth-run-locale-${process.pid}`);
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(path.join(dir, "app"), { recursive: true });
  writeFileSync(path.join(dir, "plinth.toml"), 'id = "dev.plinth.test.locale-format"\nname = "Locale Format"\nversion = "0.1.0"\npublisher = "plinth"\n');
  const textLines = cases.map((c) => `      <Text>{${c.expr}}</Text>\n`).join("");
  const source = `import { app, Screen, Text } from "plinth:ui";
import { formatNumber, formatDate, pluralCategory, formatList, compare } from "plinth:locale";

function Home() {
  return (
    <Screen title="Home">
${textLines}    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
`;
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

function checkTexts(cases, got, tag) {
  assert.equal(got.length, cases.length);
  const wrong = [];
  cases.forEach((c, i) => {
    if (got[i] !== c.web[tag]) wrong.push(`${tag} ${c.name}: got ${JSON.stringify(got[i])}, want ${JSON.stringify(c.web[tag])}`);
  });
  assert.deepEqual(wrong, []);
}

{
  const cases = readLocaleCases(readFileSync(path.join(root, "crates/plinth-compiler/tests/locale_cases.txt"), "utf-8"));
  assert.ok(cases.length >= 50, "the shared file has the cases");
  const plnt = await readPlnt(buildFormatApp(cases));
  async function startFormat(locale) {
    const app = new PlinthApp();
    const tree = new Tree();
    app.onCommit = (ops) => tree.apply(ops);
    await app.load(core, plnt.appWasm, { manifestText: plnt.manifestText, locale, log: () => {}, reportError: (s) => assert.fail(s) });
    app.init();
    return { app, tree };
  }
  for (const tag of LOCALES) {
    const { tree } = await startFormat(tag);
    checkTexts(cases, texts(tree), tag);
  }
  // A locale change formats every text again.
  const { app, tree } = await startFormat("en-US");
  for (const tag of ["de-DE", "ar-EG", "ja-JP"]) {
    app.setLocale(tag);
    checkTexts(cases, texts(tree), tag);
  }
  // Through the Worker.
  const { port1, port2 } = new MessageChannel();
  runWorker(port1);
  const worker = new WorkerApp(port2);
  const wtree = new Tree();
  worker.onCommit = (ops) => wtree.apply(ops);
  await worker.load(core, plnt.appWasm, { manifestText: plnt.manifestText, locale: "hi-IN", log: () => {}, reportError: () => {} });
  await worker.init([]);
  checkTexts(cases, texts(wtree), "hi-IN");
  port1.close();
  port2.close();
}

// -- Formatting: input that is not valid never throws ---------------------------
{
  assert.equal(formatNumberIntl("en-US", NaN, "{}"), "");
  assert.equal(formatNumberIntl("en-US", 1.5, "not json"), "1.5");
  assert.equal(formatNumberIntl("not a tag!", 1234, "{}"), "1,234");
  assert.equal(formatNumberIntl("en-US", 2, '{"style":"currency","currency":"E1"}'), "2");
  assert.equal(formatNumberIntl("en-US", 2, '{"minimumFractionDigits":99,"maximumFractionDigits":-4}'), "2.00000000000000000000");
  assert.equal(formatDateIntl("en-US", Infinity, "{}"), "");
  assert.equal(formatDateIntl("en-US", 0, '{"dateStyle":"huge"}'), "Jan 1, 1970");
  assert.equal(pluralCategoryIntl("en-US", NaN, "cardinal"), "other");
  assert.equal(formatListIntl("en-US", "[1, 2]", "conjunction"), "");
  assert.equal(compareIntl("en-US", "a", "b"), -1);
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
