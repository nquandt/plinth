// Browser test of `plinth:secrets` on the web host (core 1.13,
// docs/SECRETS.md, step 4): WebCrypto + IndexedDB (`web/secrets.js`) and the
// bridge from the sandboxed frame to the page.
//
// Part 1 runs `SecretSpace` on `idbSecretBackend` in a real page: round trip,
// a missing name, sorted names, remove, the limits (a bad name gives
// `denied:limit` also for get and remove; replace at 64 is allowed), two apps
// that do not see each other, a reload that keeps the secret, and the raw
// IndexedDB records: no plain value, a non-extractable key, a record that
// cannot move to another name (the additional authenticated data), an
// uninstall (`HubStore.kvClear`) that deletes the secrets, and
// "denied:unsupported" without WebCrypto.
// Part 2 runs examples/secrets-demo as a web export (the app runs in the
// sandboxed frame, the page keeps the secret): Save, Load, a reload, Delete.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm
// The test builds the web export itself. It needs the Playwright npm package
// (found with `require`, or in the folder of the environment variable
// PLAYWRIGHT_PATH) and a Chromium that Playwright can start. Without it the
// test prints "skipped" and ends with success.

import { createServer } from "node:http";
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import { readFile, mkdtemp, rm, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  for (const where of [undefined, process.env.PLAYWRIGHT_PATH]) {
    try {
      return require(where ? path.resolve(where) : "playwright");
    } catch {
      // try the next place
    }
  }
  return null;
}

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".plnt": "application/zip",
  ".json": "application/json",
};

/** A tiny static file server on 127.0.0.1 (a secure context, so WebCrypto works); `extra` maps a path to a page. */
function startServer(dir, extra = {}) {
  return new Promise((resolve, reject) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        if (extra[urlPath] !== undefined) {
          res.writeHead(200, { "Content-Type": "text/html" });
          res.end(extra[urlPath]);
          return;
        }
        const filePath = path.join(dir, urlPath.endsWith("/") ? `${urlPath}index.html` : urlPath);
        if (!filePath.startsWith(dir)) {
          res.writeHead(403);
          res.end();
          return;
        }
        const data = await readFile(filePath);
        res.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream" });
        res.end(data);
      } catch (err) {
        res.writeHead(404);
        res.end(String(err?.message ?? err));
      }
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
    server.on("error", reject);
  });
}

const playwright = loadPlaywright();
if (!playwright) {
  console.log("run-secrets.mjs: skipped (the Playwright npm package is not installed; set PLAYWRIGHT_PATH)");
  process.exit(0);
}

const PLAIN = "plain-VALUE-0123456789-xyz";
const browser = await playwright.chromium.launch();
let server;
let exportServer;
let tmp;
try {
  const context = await browser.newContext();
  const errors = [];

  // -- Part 1: the store, in a page ------------------------------------------
  server = await startServer(root, { "/store.html": "<!doctype html><title>secrets</title>" });
  const base = `http://127.0.0.1:${server.address().port}`;
  const page = await context.newPage();
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}/store.html`);

  /** Runs `body` in the page with `S` (secrets.js), `mk(app)` and `idb()`. */
  const inPage = (body, arg) =>
    page.evaluate(
      async ({ body, arg }) => {
        const S = await import("/web/secrets.js");
        const mk = (app) => new S.SecretSpace(S.idbSecretBackend(app));
        const idb = (name) =>
          new Promise((resolve, reject) => {
            const r = indexedDB.open(name);
            r.onsuccess = () => resolve(r.result);
            r.onerror = () => reject(r.error);
          });
        const all = (db, store) =>
          new Promise((resolve, reject) => {
            const tx = db.transaction(store, "readonly");
            const s = tx.objectStore(store);
            const keys = s.getAllKeys();
            const values = s.getAll();
            tx.oncomplete = () => resolve({ keys: keys.result, values: values.result });
            tx.onerror = () => reject(tx.error);
          });
        // eslint-disable-next-line no-new-func
        return await new Function("S", "mk", "idb", "all", "arg", `return (async () => { ${body} })()`)(S, mk, idb, all, arg);
      },
      { body, arg },
    );

  const APP = "dev.plinth.test.secrets-a";
  const OTHER = "dev.plinth.test.secrets-b";

  // Round trip, a missing name, sorted names, remove.
  const round = await inPage(
    `const s = mk(arg.app);
     const out = {};
     out.setA = await s.call("set", "token", arg.plain);
     out.get = await s.call("get", "token");
     out.missing = await s.call("get", "nope");
     out.setB = await s.call("set", "b.key", "1");
     await s.call("set", "A_key", "2");
     await s.call("set", "a-key", "3");
     out.names = await s.call("names", "");
     out.replace = await s.call("set", "token", "new");
     out.replaced = await s.call("get", "token");
     out.remove = await s.call("remove", "token");
     out.afterRemove = await s.call("get", "token");
     out.removeMissing = await s.call("remove", "token");
     await s.call("set", "token", arg.plain);
     return out;`,
    { app: APP, plain: PLAIN },
  );
  assert.deepEqual(round.setA, [true, 0, "", null]);
  assert.deepEqual(round.get, [true, 1, PLAIN, null]);
  assert.deepEqual(round.missing, [true, 0, "", null], "a missing name is not an error");
  assert.deepEqual(round.names, [true, 4, JSON.stringify(["A_key", "a-key", "b.key", "token"]), null], "names are sorted");
  assert.deepEqual(round.replaced, [true, 1, "new", null]);
  assert.deepEqual(round.remove, [true, 0, "", null]);
  assert.deepEqual(round.afterRemove, [true, 0, "", null]);
  assert.deepEqual(round.removeMissing, [true, 0, "", null]);

  // The limits.
  const limits = await inPage(
    `const s = mk(arg);
     const out = {};
     const limit = [false, 0, "", "denied:limit"];
     const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
     out.badNames = [];
     for (const bad of ["", "a b", "x".repeat(65), "caf\\u00e9", "a/b", "a\\u0000b"]) {
       const r = [await s.call("set", bad, "v"), await s.call("get", bad), await s.call("remove", bad)];
       out.badNames.push(r.every((x) => same(x, limit)));
     }
     out.name64 = (await s.call("set", "n".repeat(64), "v"))[0];
     out.value16k = (await s.call("set", "big", "x".repeat(16384)))[0];
     out.value16kRead = (await s.call("get", "big"))[2].length;
     out.value16k1 = await s.call("set", "big2", "x".repeat(16385));
     out.multibyte = await s.call("set", "big3", "\\u00e9".repeat(8193)); // 16386 bytes
     out.multibyteOk = (await s.call("set", "big3", "\\u00e9".repeat(8192)))[0]; // 16384 bytes
     await s.call("remove", "n".repeat(64));
     await s.call("remove", "big");
     await s.call("remove", "big3");
     // Fill to 64 secrets, then the 65th, a replace, and a new one after a remove.
     const before = JSON.parse((await s.call("names", ""))[2]).length;
     for (let i = before; i < 64; i++) await s.call("set", "f" + i, "v");
     out.count = JSON.parse((await s.call("names", ""))[2]).length;
     out.sixtyFifth = await s.call("set", "one-more", "v");
     out.replaceAt64 = (await s.call("set", "f" + before, "changed"))[0];
     await s.call("remove", "f" + before);
     out.afterRemove = (await s.call("set", "one-more", "v"))[0];
     return out;`,
    APP,
  );
  assert.ok(limits.badNames.every(Boolean), "a bad name gives denied:limit for set, get and remove");
  assert.equal(limits.name64, true, "a name of 64 characters is allowed");
  assert.equal(limits.value16k, true, "a value of 16 KiB is allowed");
  assert.equal(limits.value16kRead, 16384);
  assert.deepEqual(limits.value16k1, [false, 0, "", "denied:limit"]);
  assert.deepEqual(limits.multibyte, [false, 0, "", "denied:limit"], "the limit counts UTF-8 bytes");
  assert.equal(limits.multibyteOk, true);
  assert.equal(limits.count, 64);
  assert.deepEqual(limits.sixtyFifth, [false, 0, "", "denied:limit"]);
  assert.equal(limits.replaceAt64, true, "a replace at 64 is allowed");
  assert.equal(limits.afterRemove, true);

  // Two apps with the same name do not see each other.
  const iso = await inPage(
    `const a = mk(arg.a);
     const b = mk(arg.b);
     await b.call("set", "token", "value-of-b");
     const out = {
       a: await a.call("get", "token"),
       b: await b.call("get", "token"),
       bNames: await b.call("names", ""),
     };
     await a.call("remove", "token");
     out.bStill = await b.call("get", "token");
     await a.call("set", "token", arg.plain);
     return out;`,
    { a: APP, b: OTHER, plain: PLAIN },
  );
  assert.deepEqual(iso.a, [true, 1, PLAIN, null]);
  assert.deepEqual(iso.b, [true, 1, "value-of-b", null]);
  assert.deepEqual(iso.bNames, [true, 1, '["token"]', null]);
  assert.deepEqual(iso.bStill, [true, 1, "value-of-b", null], "a remove in one app never touches the other");

  // A reload keeps the secret (the key and the values are in IndexedDB).
  await page.reload();
  const again = await inPage(`return await mk(arg).call("get", "token");`, APP);
  assert.deepEqual(again, [true, 1, PLAIN, null], "a reload keeps the secret");

  // The raw records: no plain value; the key cannot be exported.
  const raw = await inPage(
    `const db = await idb("plinth-secrets");
     const values = await all(db, "values");
     const keys = await all(db, "keys");
     const key = keys.values[0];
     let exportError = null;
     try { await crypto.subtle.exportKey("raw", key); } catch (e) { exportError = e.name; }
     const text = (buf) => String.fromCharCode(...new Uint8Array(buf));
     const dump = JSON.stringify(values.values.map((v) => ({ iv: text(v.iv), data: text(v.data) })));
     const ivs = values.values.map((v) => Array.from(new Uint8Array(v.iv)).join(","));
     const shapes = values.values.map((v) => [v.iv instanceof ArrayBuffer, v.iv.byteLength, v.data instanceof ArrayBuffer]);
     const sizes = values.values.map((v, i) => [values.keys[i][2], v.data.byteLength]);
     db.close();
     return { keyCount: keys.values.length, keyType: key.type, keyExtractable: key.extractable, algorithm: key.algorithm, exportError,
       plainInRecords: dump.includes(arg), plainBytes: values.values.some((v) => new Uint8Array(v.data).join(",").includes(Array.from(new TextEncoder().encode(arg)).join(","))),
       recordKeys: values.keys, ivs, shapes, sizes, n: values.values.length };`,
    PLAIN,
  );
  assert.equal(raw.keyCount, 1, "one key for the profile");
  assert.equal(raw.keyType, "secret");
  assert.equal(raw.keyExtractable, false);
  assert.equal(raw.algorithm.name, "AES-GCM");
  assert.equal(raw.algorithm.length, 256);
  assert.ok(raw.exportError, "the key cannot be exported");
  assert.ok(raw.n >= 3);
  assert.equal(raw.plainInRecords, false, "the stored records do not hold the plain value");
  assert.equal(raw.plainBytes, false);
  assert.ok(raw.shapes.every(([a, len, b]) => a && len === 12 && b), "iv: 12-byte ArrayBuffer, data: ArrayBuffer");
  assert.equal(new Set(raw.ivs).size, raw.ivs.length, "each write has its own random IV");
  assert.ok(raw.recordKeys.every((k) => Array.isArray(k) && k.length === 3 && k[1] === "default"), "keys: [app, instance, name]");
  assert.ok(raw.sizes.some(([name, size]) => name === "token" && size === new TextEncoder().encode(PLAIN).length + 16), "AES-GCM: value + 16-byte tag");

  // A record moved to another name does not decrypt (the additional authenticated data).
  const moved = await inPage(
    `const db = await idb("plinth-secrets");
     const rec = await new Promise((resolve) => { const r = db.transaction("values").objectStore("values").get([arg.app, "default", "token"]); r.onsuccess = () => resolve(r.result); });
     await new Promise((resolve, reject) => { const tx = db.transaction("values", "readwrite"); tx.objectStore("values").put(rec, [arg.app, "default", "moved"]); tx.oncomplete = resolve; tx.onerror = () => reject(tx.error); });
     db.close();
     const s = mk(arg.app);
     const out = { moved: await s.call("get", "moved"), original: await s.call("get", "token") };
     await s.call("remove", "moved");
     return out;`,
    { app: APP.toLowerCase() },
  );
  assert.deepEqual(moved.moved, [false, 0, "", "failed"], "a moved record fails, and the error holds no value");
  assert.deepEqual(moved.original, [true, 1, PLAIN, null]);

  // An uninstall deletes the secrets of the app (and only that app).
  const gone = await inPage(
    `const { HubStore, memoryStorage } = await import("/web/hub-storage.js");
     const store = new HubStore(memoryStorage(), memoryStorage());
     await store.kvClear(arg.a);
     const a = await mk(arg.a).call("names", "");
     const b = await mk(arg.b).call("names", "");
     return { a, b };`,
    { a: APP, b: OTHER },
  );
  assert.deepEqual(gone.a, [true, 0, "[]", null], "kvClear deletes the secrets of the app");
  assert.deepEqual(gone.b, [true, 1, '["token"]', null], "and not those of another app");

  // No WebCrypto or no IndexedDB: denied:unsupported, never a weaker store.
  const unsupported = await inPage(
    `const s = new S.SecretSpace(S.idbSecretBackend("x", indexedDB, null));
     const t = new S.SecretSpace(S.idbSecretBackend("x", null, crypto.subtle));
     return [await s.call("set", "k", "v"), await t.call("get", "k"), await s.call("names", ""), await s.call("get", "bad name")];`,
  );
  assert.deepEqual(unsupported[0], [false, 0, "", "denied:unsupported"]);
  assert.deepEqual(unsupported[1], [false, 0, "", "denied:unsupported"]);
  assert.deepEqual(unsupported[2], [false, 0, "", "denied:unsupported"]);
  assert.deepEqual(unsupported[3], [false, 0, "", "denied:limit"]);

  await inPage(`await mk(arg.a).call("remove", "token"); await mk(arg.b).call("remove", "token");`, { a: APP, b: OTHER });
  console.log("secrets store: round trip, sorted names, limits, isolation, reload, encrypted records, AAD, uninstall, unsupported");

  // -- Part 2: secrets-demo in the sandboxed frame ---------------------------
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  tmp = await mkdtemp(path.join(tmpdir(), "plinth-secrets-"));
  const folder = path.join(tmp, "web");
  execFileSync(exe, ["build", path.join(root, "examples/secrets-demo"), "--target", "web", "--out", folder], { stdio: "ignore" });
  // The export carries the web host of the CLI; the test runs this tree's.
  await copyFile(path.join(root, "web/plinth.js"), path.join(folder, "plinth.js"));
  exportServer = await startServer(folder);
  const appBase = `http://127.0.0.1:${exportServer.address().port}`;
  const app = await context.newPage();
  app.on("pageerror", (e) => errors.push(String(e)));
  const open = async () => {
    await app.goto(`${appBase}/`);
    const frame = app.frameLocator("iframe");
    await frame.getByRole("button", { name: "Save" }).waitFor({ timeout: 30000 });
    return frame;
  };
  const status = async (frame, text) => {
    await frame.getByText(text, { exact: false }).first().waitFor({ timeout: 10000 });
  };
  let frame = await open();
  assert.equal(await app.evaluate(() => location.origin !== "null"), true);
  await frame.getByLabel("Value").fill(PLAIN);
  await frame.getByRole("button", { name: "Save" }).click();
  await status(frame, "Saved api-token.");
  await frame.getByLabel("Value").fill("");
  await frame.getByRole("button", { name: "Load" }).click();
  await status(frame, "Loaded api-token.");
  assert.equal(await frame.getByLabel("Value").inputValue(), PLAIN, "the frame gets the value back from the page");

  // The page holds the encrypted record, not the plain text.
  const frameRaw = await app.evaluate(async (plain) => {
    const db = await new Promise((resolve, reject) => {
      const r = indexedDB.open("plinth-secrets");
      r.onsuccess = () => resolve(r.result);
      r.onerror = () => reject(r.error);
    });
    const values = await new Promise((resolve) => {
      const s = db.transaction("values").objectStore("values");
      const k = s.getAllKeys();
      const v = s.getAll();
      s.transaction.oncomplete = () => resolve({ keys: k.result, values: v.result });
    });
    db.close();
    const dump = JSON.stringify(values.values.map((v) => [...new Uint8Array(v.data)].map((c) => String.fromCharCode(c)).join("")));
    return { keys: values.keys, plain: dump.includes(plain), local: JSON.stringify({ ...localStorage }).includes(plain) };
  }, PLAIN);
  assert.deepEqual(frameRaw.keys, [["dev.plinth.examples.secrets-demo", "default", "api-token"]]);
  assert.equal(frameRaw.plain, false, "the frame's secret is encrypted in the page's IndexedDB");
  assert.equal(frameRaw.local, false, "and not in localStorage");

  // A reload keeps it; Delete removes it.
  frame = await open();
  await frame.getByRole("button", { name: "Load" }).click();
  await status(frame, "Loaded api-token.");
  assert.equal(await frame.getByLabel("Value").inputValue(), PLAIN, "a reload keeps the secret");
  await frame.getByRole("button", { name: "Delete" }).click();
  await status(frame, "Deleted api-token.");
  await frame.getByRole("button", { name: "Load" }).click();
  await status(frame, "There is no secret named api-token.");
  // A name that breaks a limit: the frame answers denied:limit.
  await frame.getByLabel("Name").fill("bad name");
  await frame.getByRole("button", { name: "Save" }).click();
  await status(frame, "Not saved: denied:limit");
  console.log("secrets-demo in the sandboxed frame: save, load, encrypted in the page, reload, delete, denied:limit");

  assert.deepEqual(errors, [], "no page error");
  console.log("run-secrets.mjs: ok");
} finally {
  await browser.close();
  server?.close();
  exportServer?.close();
  if (tmp) await rm(tmp, { recursive: true, force: true }).catch(() => {});
}
