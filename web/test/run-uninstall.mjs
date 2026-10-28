// Browser test of "uninstall with data" on the web host (docs/STORAGE.md S13,
// `HubStore.uninstall` in web/hub-storage.js, `HubHost.uninstall` in
// web/hub-host.js).
//
// One real page (Chromium with OPFS and IndexedDB) keeps data for two apps in
// every store of the web host: kv (localStorage), files (IndexedDB
// `plinth-files`), a store.sql database (OPFS, `.plinth-sql`, in the SQLite
// Worker of the page), secrets (IndexedDB `plinth-secrets`, encrypted) and a
// grant record. The uninstall of app A must delete all of it, in a reload too,
// and must not change anything of app B. Then: a failing step (all other steps
// still run, the grant records stay, the call rejects), and `HubHost.uninstall`
// (refuses while the app runs, refuses without an eraser, keeps the state when
// the eraser fails, clears the Hub's records, keeps the block list, and
// `install` shows the app again as a new app). Then the `plinth:hub` call
// `uninstall(id, keepData)` of the Hub app (core 1.13): the Hub app itself is
// refused ("the Hub cannot uninstall itself"), and the Hub app frame
// (`FrameHub`) sends the call to the page (`hub-call`), which erases the data
// and answers with a new snapshot; the bridge accepts only the right shape.
//
// Needs the Playwright npm package (found with `require`, or in the folder of
// the environment variable PLAYWRIGHT_PATH) and a Chromium that Playwright
// can start. Without it the test prints "skipped" and ends with success.

import { createServer } from "node:http";
import { createRequire } from "node:module";
import { readFile } from "node:fs/promises";
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

const MIME = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".wasm": "application/wasm", ".json": "application/json" };

function startServer(dir, pages) {
  return new Promise((resolve, reject) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        if (pages[urlPath] !== undefined) {
          res.writeHead(200, { "Content-Type": "text/html" });
          res.end(pages[urlPath]);
          return;
        }
        const filePath = path.join(dir, urlPath);
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
  console.log("run-uninstall.mjs: skipped (the Playwright npm package is not installed; set PLAYWRIGHT_PATH)");
  process.exit(0);
}

const A = "dev.plinth.test.uninstall-a";
const B = "dev.plinth.test.uninstall-b";

const browser = await playwright.chromium.launch();
let server;
try {
  const context = await browser.newContext();
  const errors = [];
  server = await startServer(root, { "/store.html": "<!doctype html><title>uninstall</title>" });
  const base = `http://127.0.0.1:${server.address().port}`;
  const page = await context.newPage();
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}/store.html`);

  /**
   * Runs `body` in the page. Helpers: `H` (hub-storage.js), `F` (files.js),
   * `S` (secrets.js), `Q` (sql.js), `store` (a HubStore on localStorage),
   * `sqlHost` (one SqlWorkerHost for the page, kept across calls), `files(app)`,
   * `secrets(app)`, `sql(app, kind, statement, params)`.
   */
  const inPage = (body, arg) =>
    page.evaluate(
      async ({ body, arg }) => {
        const H = await import("/web/hub-storage.js");
        const F = await import("/web/files.js");
        const S = await import("/web/secrets.js");
        const Q = await import("/web/sql.js");
        window.__sqlHost ??= new Q.SqlWorkerHost(new URL("/web/sql-worker.js", location.href));
        const sqlHost = window.__sqlHost;
        const store = new H.HubStore(localStorage, sessionStorage);
        const files = (app) => new F.FileSpace(F.idbFileBackend(app));
        const secrets = (app) => new S.SecretSpace(S.idbSecretBackend(app));
        const sql = (app, kind, statement, params = "") => sqlHost.call(app, kind, statement, params);
        const rawFileCount = (app) =>
          new Promise((resolve, reject) => {
            const r = indexedDB.open("plinth-files", 1);
            r.onupgradeneeded = () => r.result.createObjectStore("files");
            r.onerror = () => reject(r.error);
            r.onsuccess = () => {
              const db = r.result;
              const q = db.transaction("files").objectStore("files").count(IDBKeyRange.bound([app, ""], [app, []]));
              q.onsuccess = () => {
                db.close();
                resolve(q.result);
              };
            };
          });
        // eslint-disable-next-line no-new-func
        return await new Function("H", "F", "S", "Q", "store", "sqlHost", "files", "secrets", "sql", "rawFileCount", "arg", `return (async () => { ${body} })()`)(
          H,
          F,
          S,
          Q,
          store,
          sqlHost,
          files,
          secrets,
          sql,
          rawFileCount,
          arg,
        );
      },
      { body, arg },
    );

  // -- Data for two apps, in every store ---------------------------------------
  const made = await inPage(
    `const out = {};
     for (const app of arg) {
       store.kvSet(app, "theme", "dark-" + app);
       store.setDataVersion(app, "1.0.0");
       store.saveGrants(app, { id: app, name: app, grants: {} }, { grants: {} });
       out["file-" + app] = await files(app).call("write", "notes/a.md", "hello " + app);
       await secrets(app).call("set", "token", "secret-" + app);
       out["migrate-" + app] = (await sql(app, 3, "", JSON.stringify(["CREATE TABLE t (v TEXT)"])))[0];
       out["insert-" + app] = (await sql(app, 0, "INSERT INTO t VALUES (?)", JSON.stringify(["row-" + app])))[0];
     }
     out.inMemory = sqlHost.inMemory;
     return out;`,
    [A, B],
  );
  for (const app of [A, B]) {
    assert.deepEqual(made["file-" + app], [true, 0, "", null]);
    assert.equal(made["migrate-" + app], true);
    assert.equal(made["insert-" + app], true);
  }
  assert.notEqual(made.inMemory, true, "this browser keeps the SQL databases in OPFS, so the test is real");

  // The state of one app, read from the stores.
  const readApp = (app) =>
    inPage(
      `const rows = await sql(arg, 1, "SELECT v FROM t");
       const tables = await sql(arg, 1, "SELECT name FROM sqlite_master");
       return {
         kv: store.kvSnapshot(arg),
         dataVersion: store.dataVersion(arg),
         grants: store.grants(arg),
         sessionGrants: store.sessionGrants(arg),
         fileRead: await files(arg).call("read", "notes/a.md"),
         fileRaw: await rawFileCount(arg.toLowerCase()),
         secretNames: (await secrets(arg).call("names", ""))[2],
         rows: rows[0] ? rows[2] : rows[3],
         tables: tables[2],
       };`,
      app,
    );

  const beforeA = await readApp(A);
  const beforeB = await readApp(B);
  assert.deepEqual(beforeA.kv, { theme: `dark-${A}` });
  assert.equal(beforeA.fileRead[2], `hello ${A}`);
  assert.equal(beforeA.fileRaw, 1);
  assert.equal(beforeA.secretNames, '["token"]');
  assert.equal(beforeA.rows, `[{"v":"row-${A}"}]`);
  assert.ok(beforeA.grants && beforeA.sessionGrants);

  // -- The uninstall of A -------------------------------------------------------
  await inPage(`await store.uninstall(arg, { sql: sqlHost });`, A);
  const checkGone = (after) => {
    assert.deepEqual(after.kv, {}, "kv data is gone");
    assert.equal(after.dataVersion, null, "the data version is gone");
    assert.equal(after.grants, null, "the grant record is gone");
    assert.equal(after.sessionGrants, null, "the session grant record is gone");
    assert.equal(after.fileRead[3], "not-found", "the file is gone");
    assert.equal(after.fileRaw, 0, "no IndexedDB record of the app is left");
    assert.equal(after.secretNames, "[]", "the secrets are gone");
    assert.equal(after.tables, "[]", "the SQL database is new and empty");
  };
  checkGone(await readApp(A));
  assert.deepEqual(await readApp(B), beforeB, "app B did not change in any store");
  console.log("uninstall of A: kv, data version, grants, files, secrets and the OPFS database are gone; B is unchanged");

  // The same after a reload (the data is really deleted, not only hidden).
  await page.reload();
  let reloaded;
  for (let attempt = 0; attempt < 10; attempt++) {
    try {
      reloaded = await readApp(A);
      break;
    } catch (err) {
      if (attempt === 9) throw err;
      await new Promise((r) => setTimeout(r, 300));
    }
  }
  checkGone(reloaded);
  assert.deepEqual(await readApp(B), beforeB, "B is unchanged after the reload");
  console.log("after a reload the data of A is still gone");

  // -- A failing step: all steps run, the call rejects, the grants stay ----------
  const failing = await inPage(
    `store.kvSet(arg, "k", "v");
     store.saveGrants(arg, { id: arg, name: arg, grants: {} }, null);
     await files(arg).call("write", "f.txt", "x");
     const bad = new H.HubStore(localStorage, sessionStorage, undefined, async () => { throw new Error("idb is locked"); });
     let message = null;
     try { await bad.uninstall(arg, { sql: sqlHost }); } catch (e) { message = e.message; }
     const afterFail = { message, kv: store.kvSnapshot(arg), grants: store.grants(arg) !== null, fileRaw: await rawFileCount(arg) };
     await store.uninstall(arg, { sql: sqlHost });
     return { afterFail, kv: store.kvSnapshot(arg), grants: store.grants(arg), fileRaw: await rawFileCount(arg) };`,
    A.toLowerCase(),
  );
  assert.match(failing.afterFail.message, /could not delete all data of .* \(files: idb is locked\)/);
  assert.deepEqual(failing.afterFail.kv, {}, "the other steps ran");
  assert.equal(failing.afterFail.grants, true, "the grant record stays after a failure");
  assert.equal(failing.afterFail.fileRaw, 1);
  assert.deepEqual(failing.kv, {});
  assert.equal(failing.grants, null);
  assert.equal(failing.fileRaw, 0, "the second try deleted everything");
  console.log("a failing step: the other steps run, the grant records stay, a second try works");

  // -- HubHost.uninstall ----------------------------------------------------------
  const hostChecks = await inPage(
    `const { HubHost, memoryPersist } = await import("/web/hub-host.js");
     const entry = { id: arg, name: "T", publisher: "me", source: "s", registry: {}, versions: [{ version: "1.0.0", sha256: "x", core: null, signer: "", capabilities: ["store.kv"], rationales: {} }] };
     const erased = [];
     let running = true;
     const persist = memoryPersist();
     const host = new HubHost({ persist, erase: async (id) => { erased.push(id); }, isRunning: () => running });
     host.catalog.set(arg, entry);
     host.grantLowRisk(arg, entry.versions[0]);
     host.state.known[arg] = "1.0.0";
     host.state.pinned[arg] = "1.0.0";
     host.state.appGroups[arg] = ["Work"];
     host.state.blocks.apps.push(arg);
     const out = {};
     try { await host.uninstall(arg); } catch (e) { out.running = e.message; }
     running = false;
     out.erasedWhileRunning = erased.length;
     // An eraser that fails: nothing changes.
     const failing = new HubHost({ persist, erase: async () => { throw new Error("disk"); } });
     failing.catalog.set(arg, entry);
     failing.state = structuredClone(host.state);
     try { await failing.uninstall(arg); } catch (e) { out.failing = e.message; }
     out.failingStillListed = failing.get(arg) !== null;
     out.failingGrants = Object.keys(failing.grants(arg));
     // No eraser: refuses; keep data: hides only.
     const bare = new HubHost({ persist });
     bare.catalog.set(arg, entry);
     bare.state = structuredClone(host.state);
     try { await bare.uninstall(arg); } catch (e) { out.bare = e.message; }
     await bare.uninstall(arg, { keepData: true });
     out.keepHidden = bare.get(arg) === null;
     out.keepGrants = Object.keys(bare.grants(arg));
     // The real uninstall.
     await host.uninstall(arg);
     out.erased = erased;
     out.hidden = host.get(arg) === null;
     out.grants = Object.keys(host.grants(arg));
     out.known = host.state.known[arg] ?? null;
     out.pinned = host.state.pinned[arg] ?? null;
     out.groups = host.groupsOf(arg);
     out.blocked = host.isAppBlocked(arg);
     out.saved = JSON.stringify((await persist.load()).removed);
     // Install shows the app again, as a new app (a blocked app does not install).
     host.unblock(arg);
     await host.install(arg);
     out.back = host.get(arg) !== null;
     out.newKnown = host.state.known[arg];
     out.newGrants = Object.keys(host.grants(arg));
     return out;`,
    "dev.plinth.test.uninstall-host",
  );
  assert.match(hostChecks.running, /is running; close it first/);
  assert.equal(hostChecks.erasedWhileRunning, 0, "nothing is deleted while the app runs");
  assert.equal(hostChecks.failing, "disk");
  assert.equal(hostChecks.failingStillListed, true, "an eraser that fails changes nothing");
  assert.deepEqual(hostChecks.failingGrants, ["store.kv"]);
  assert.equal(hostChecks.bare, "this host cannot delete app data");
  assert.equal(hostChecks.keepHidden, true);
  assert.deepEqual(hostChecks.keepGrants, ["store.kv"], "keep data keeps the grants");
  assert.deepEqual(hostChecks.erased, ["dev.plinth.test.uninstall-host"]);
  assert.equal(hostChecks.hidden, true);
  assert.deepEqual(hostChecks.grants, [], "the Hub's grant records are gone");
  assert.equal(hostChecks.known, null);
  assert.equal(hostChecks.pinned, null);
  assert.deepEqual(hostChecks.groups, []);
  assert.equal(hostChecks.blocked, true, "the block list is not changed");
  assert.match(hostChecks.saved, /uninstall-host/);
  assert.equal(hostChecks.back, true);
  assert.equal(hostChecks.newKnown, "1.0.0");
  assert.deepEqual(hostChecks.newGrants, ["store.kv"], "the low-risk default is granted again, as for a new app");
  console.log("HubHost.uninstall: refuses while running and without an eraser, keeps state when the eraser fails, clears the records, install starts new");

  // -- The Hub app frame: `uninstall` goes to the page (the `hub-call` bridge) ----
  const framed = await inPage(
    `const { HubHost, FrameHub, memoryPersist } = await import("/web/hub-host.js");
     const { checkFrameMessage, CHANNEL } = H;
     const hubId = "dev.plinth.test.hub-app";
     const entry = (id) => ({ id, name: id, publisher: "me", source: "s", registry: {}, versions: [{ version: "1.0.0", sha256: "x", core: null, signer: "", capabilities: ["store.kv"], rationales: {} }] });
     const erased = [];
     let running = new Set();
     const host = new HubHost({ persist: memoryPersist(), hide: [hubId], erase: async (id) => { erased.push(id); }, isRunning: (id) => running.has(id) });
     for (const id of [arg + "-1", arg + "-2", arg + "-3"]) {
       host.catalog.set(id, entry(id));
       host.grantLowRisk(id, entry(id).versions[0]);
       host.state.known[id] = "1.0.0";
     }
     // The bridge of frame-host.js: checks the message, calls the page host.
     const calls = [];
     const ask = async (type, { method, args }) => {
       const msg = checkFrameMessage({ channel: CHANNEL, type, id: 1, method, args });
       if (!msg) return { ok: false, error: "refused by the bridge" };
       calls.push([method, ...args]);
       try {
         const value = msg.method === "uninstall" ? await host.uninstall(msg.args[0], { keepData: msg.args[1] === true }) : await host[msg.method](...msg.args);
         return { ok: true, value: value ?? null, snapshot: host.snapshot() };
       } catch (err) {
         return { ok: false, error: err.message, snapshot: host.snapshot() };
       }
     };
     const frame = new FrameHub(host.snapshot(), { send: () => {}, ask });
     const out = {};
     // With the data.
     out.deleteResult = await frame.uninstall(arg + "-1", { keepData: false });
     out.erased = [...erased];
     out.frameHidden = frame.get(arg + "-1") === null;
     out.pageHidden = host.get(arg + "-1") === null;
     // Keep my data: no erase.
     await frame.uninstall(arg + "-2", { keepData: true });
     out.erasedAfterKeep = [...erased];
     out.keepHidden = frame.get(arg + "-2") === null;
     out.keepGrants = Object.keys(host.grants(arg + "-2"));
     // An app that runs: the text of the page reaches the frame; nothing changes.
     running.add(arg + "-3");
     try { await frame.uninstall(arg + "-3"); } catch (e) { out.runningError = e.message; }
     out.runningStillListed = frame.get(arg + "-3") !== null && host.get(arg + "-3") !== null;
     // The Hub app itself (the page hides it) and an unknown app.
     try { await frame.uninstall(hubId); } catch (e) { out.selfError = e.message; }
     try { await frame.uninstall("dev.plinth.test.nothing"); } catch (e) { out.unknownError = e.message; }
     out.calls = calls.map((c) => c.join("|"));
     // The bridge shape: id and a boolean.
     const ok = (args) => !!checkFrameMessage({ channel: CHANNEL, type: "hub-call", id: 1, method: "uninstall", args });
     out.shape = [ok(["a", true]), ok(["a", false]), ok(["a"]), ok(["a", "yes"]), ok([1, true]), ok(["a", true, "b"]), ok([])];
     return out;`,
    "dev.plinth.test.frame-uninstall",
  );
  assert.equal(framed.deleteResult, null);
  assert.deepEqual(framed.erased, ["dev.plinth.test.frame-uninstall-1"], "the page erased the data");
  assert.equal(framed.frameHidden, true, "the frame has the new state");
  assert.equal(framed.pageHidden, true);
  assert.deepEqual(framed.erasedAfterKeep, framed.erased, "keep my data erases nothing");
  assert.equal(framed.keepHidden, true);
  assert.deepEqual(framed.keepGrants, ["store.kv"]);
  assert.match(framed.runningError, /is running; close it first/);
  assert.equal(framed.runningStillListed, true);
  assert.equal(framed.selfError, "the Hub cannot uninstall itself");
  assert.match(framed.unknownError, /is not in the library/);
  assert.deepEqual(framed.shape, [true, true, false, false, false, false, false], "the bridge takes an app id and a boolean only");
  console.log("Hub app frame: uninstall goes to the page, keep data erases nothing, a running app and the Hub itself are refused");

  // Clean up the data of the test apps.
  await inPage(`for (const app of arg) await store.uninstall(app, { sql: sqlHost });`, [A, B]);
  assert.deepEqual(errors, [], "no page error");
  console.log("run-uninstall.mjs: ok");
} finally {
  await browser.close();
  server?.close();
}
