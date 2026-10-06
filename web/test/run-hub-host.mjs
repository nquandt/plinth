// Node test (docs/web-hub.md): the `plinth:hub` backend of the web host
// (`hub-host.js`) against a registry that `plinth registry serve --web`
// serves, and the Hub app (`examples/hub`) itself on top of it, with no DOM.
//
// - The server: the redirect to the bootstrap page, `hub-config.json` with
//   the trusted keys from PLINTH_HUB_TRUSTED_KEYS.
// - search, install (digest and signature checks), listApps/appInfo with
//   the fields of the desktop `HubService`, risk-level defaults, consent,
//   the refused capabilities, groups, pins, blocks, publisher blocks,
//   remove, and the state after a reload from the store.
// - checkUpdates and update with a new capability: re-consent; until then
//   the previous version runs.
// - The Hub app runs with `hub.manage` from the host: its library shows the
//   installed apps, and its Open button asks the host to launch the app.
//   Without the host, or with `hub.manage` refused, its calls are denied.
//
// Build first (scripts/ci-local.sh does this):
//   cargo build -p plinth-cli; plinth core export target/core.wasm
//   plinth build examples/<app>   # counter, notes, utility, hub

import { spawn, execFileSync } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { loadRegistry, searchApps, pickCore } from "../registry-client.js";
import { HubHost, memoryPersist, riskOf, isNewer } from "../hub-host.js";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
const APPS = ["counter", "notes", "utility", "hub"];

/** Starts `plinth registry serve <dir> --web --port 0` and returns the base URL from its output. */
function startServer(dir, env) {
  const child = spawn(exe, ["registry", "serve", dir, "--web", "--port", "0"], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true, env });
  return new Promise((resolve, reject) => {
    let out = "";
    const timer = setTimeout(() => reject(new Error(`the server did not start: ${out}`)), 15000);
    child.stdout.on("data", (chunk) => {
      out += chunk;
      const m = /on (http:\/\/127\.0\.0\.1:\d+)/.exec(out);
      if (m) {
        clearTimeout(timer);
        resolve({ child, base: `${m[1]}/` });
      }
    });
    child.on("exit", (code) => reject(new Error(`the server stopped (${code}): ${out}`)));
  });
}

/** Runs a `.plnt` with the web host and tracks its UI (texts, labels, titles and press handlers). */
async function runApp(coreBytes, plnt, opts) {
  const { appWasm, manifestText } = await readPlnt(plnt);
  const app = new PlinthApp();
  const text = new Map();
  const names = new Map(); // node id -> label or title
  const listen = new Map(); // node id -> Map<event, handler>
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") text.set(op.id, String(op.value));
      else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && (op.prop === 3 || op.prop === 1)) names.set(op.id, op.value);
    }
  };
  await app.load(coreBytes, appWasm, { log: () => {}, manifestText, ...opts });
  app.init([]);
  const settle = () => new Promise((r) => setTimeout(r, 50));
  return {
    app,
    allText: () => [...text.values(), ...names.values()].join(" | "),
    press(name) {
      for (const [id, byEvent] of listen) {
        if (byEvent.has(1) && names.get(id) === name) {
          app.onEvent({ kind: "ui", handler: byEvent.get(1), event: 1, value: null });
          return settle();
        }
      }
      throw new Error(`nothing to press with the name "${name}"`);
    },
  };
}

async function waitFor(fn, what, ms = 5000) {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > deadline) throw new Error(`timed out: ${what}`);
    await new Promise((r) => setTimeout(r, 25));
  }
}

async function main() {
  const dir = mkdtempSync(path.join(tmpdir(), "plinth-web-hub-host-"));
  const reg = path.join(dir, "registry");
  let server = null;
  try {
    // A throwaway publisher key named like the Hub manifest's publisher.
    const env = { ...process.env, PLINTH_PUBLISHER_DIR: path.join(dir, "publisher") };
    execFileSync(exe, ["publisher", "init", "--name", "you"], { env, stdio: "ignore" });
    const key = execFileSync(exe, ["publisher", "show"], { env, encoding: "utf8" }).trim().split(/\s+/).pop();
    assert.match(key, /^ed25519:/);
    mkdirSync(reg);
    for (const app of APPS) {
      const pkg = path.join(root, "examples", app, "dist", `${app}.plnt`);
      assert.ok(existsSync(pkg), `build ${app} first: ${pkg}`);
      copyFileSync(pkg, path.join(reg, `${app}.plnt`));
    }
    execFileSync(exe, ["sign", path.join(reg, "hub.plnt")], { env, stdio: "ignore" });
    copyFileSync(path.join(reg, "hub.plnt"), path.join(dir, "hub.plnt"));
    execFileSync(exe, ["registry", "build", reg, "--with-core"], { stdio: "ignore" });
    server = await startServer(reg, { ...process.env, PLINTH_HUB_TRUSTED_KEYS: key });
    const { base } = server;

    // The server: the bootstrap page and the trusted keys.
    const landing = await fetch(base, { redirect: "manual" });
    assert.equal(landing.status, 302);
    assert.equal(landing.headers.get("location"), "/web/hub.html");
    const config = await (await fetch(new URL("/web/hub-config.json", base))).json();
    assert.deepEqual(config.trustedKeys, [key]);
    assert.equal(config.hub, "dev.plinth.hub");

    // The registry client.
    const registry = await loadRegistry(base);
    assert.deepEqual(
      registry.apps.map((a) => a.name),
      ["Counter", "Notes", "Plinth Hub", "Utility"],
    );
    assert.equal(registry.cores.length, 1, "--with-core gives one core");
    assert.deepEqual(searchApps(registry.apps, "NOTE").map((a) => a.id), ["dev.plinth.examples.notes"]);
    assert.equal(searchApps(registry.apps, "").length, 4);
    assert.equal(pickCore([{ version: "1.2" }, { version: "1.9" }, { version: "2.0" }], "1.3").version, "1.9");
    assert.equal(pickCore([{ version: "1.2" }], "1.3"), null);
    // The host's risk table agrees with the registry labels (the shared capability map).
    for (const app of registry.apps) for (const label of app.labels ?? []) assert.equal(riskOf(label.name), label.risk, `${app.id}: ${label.name}`);
    assert.equal(isNewer("0.10.0", "0.9.1"), true);
    assert.equal(isNewer("1.0.0", "1.0.0"), false);

    // The host: search and install.
    const persist = memoryPersist();
    const launches = [];
    let host = await new HubHost({ persist, onLaunch: (id) => launches.push(id) }).load();
    host.addSource("local", base);
    const found = JSON.parse(await host.search("not"));
    assert.deepEqual(found.errors, []);
    assert.deepEqual(found.hits, [{ id: "dev.plinth.examples.notes", name: "Notes", version: "0.1.0", description: "", source: "local" }]);
    for (const id of ["dev.plinth.examples.counter", "dev.plinth.examples.notes", "dev.plinth.examples.utility"]) assert.equal(await host.install(id), id);
    await assert.rejects(host.install("com.example.none"), /no configured source lists com\.example\.none/);

    const apps = JSON.parse(host.listAppsJson());
    assert.deepEqual(apps.map((a) => a.id), ["dev.plinth.examples.counter", "dev.plinth.examples.notes", "dev.plinth.examples.utility"]);
    const notes = JSON.parse(host.appInfoJson("dev.plinth.examples.notes"));
    // The fields of `HubService::app_json` (crates/plinth-hub/src/lib.rs).
    assert.deepEqual(Object.keys(notes).sort(), [
      "blocked",
      "capabilities",
      "groups",
      "id",
      "name",
      "pinned",
      "publisher",
      "publisherBlocked",
      "signer",
      "source",
      "update",
      "updateCapabilities",
      "version",
      "versions",
    ]);
    assert.equal(notes.publisher, "plinth");
    assert.equal(notes.source, "local");
    assert.equal(notes.signer, "");
    assert.deepEqual(notes.capabilities, [
      { name: "store.kv", risk: "low", description: "save data on this device", rationale: "Save your notes on this device.", decided: true, allowed: true, byDefault: true },
    ]);
    assert.deepEqual(notes.versions, [{ version: "0.1.0", signer: "", capabilities: ["store.kv"] }]);
    assert.throws(() => host.appInfoJson("com.example.none"), /not in the library/);

    // Consent: clipboard.read (medium) needs a decision, clipboard.write (low) does not.
    const utilityId = "dev.plinth.examples.utility";
    const utilityV = host.runnableVersion(utilityId);
    assert.deepEqual(host.needsConsent(utilityId, utilityV.capabilities), ["clipboard.read"]);
    assert.deepEqual(host.consentRows(utilityId, utilityV), [
      { name: "clipboard.read", risk: "medium", description: "read the clipboard", rationale: "Paste text into the input box." },
    ]);
    assert.deepEqual(host.refusedFor(utilityId, utilityV.capabilities), ["clipboard.read"], "undecided is refused");
    host.recordConsent(utilityId, utilityV.version, { "clipboard.read": false });
    assert.deepEqual(host.needsConsent(utilityId, utilityV.capabilities), []);
    host.setGrant(utilityId, "clipboard.write", false);
    assert.deepEqual(host.refusedFor(utilityId, utilityV.capabilities).sort(), ["clipboard.read", "clipboard.write"]);
    assert.deepEqual(host.refusedFor("x", ["hub.manage"]), ["hub.manage"], "a launched app never gets hub.manage");

    // Groups, pins, blocks, remove, launch.
    host.createGroup("Work");
    assert.throws(() => host.createGroup("  "), /needs a name/);
    host.setGroup("dev.plinth.examples.counter", "Work", true);
    host.setGroup("dev.plinth.examples.counter", "Play", true);
    assert.deepEqual(JSON.parse(host.listGroupsJson()), ["Work", "Play"]);
    assert.deepEqual(JSON.parse(host.appInfoJson("dev.plinth.examples.counter")).groups, ["Work", "Play"]);
    host.setGroup("dev.plinth.examples.counter", "Play", false);
    assert.throws(() => host.pin("dev.plinth.examples.counter", "9.9.9"), /no version 9\.9\.9/);
    host.pin("dev.plinth.examples.counter", "0.1.0");
    assert.equal(JSON.parse(host.appInfoJson("dev.plinth.examples.counter")).pinned, "0.1.0");
    host.pin("dev.plinth.examples.counter", "");
    host.block("dev.plinth.examples.counter");
    assert.equal(host.isBlocked("dev.plinth.examples.counter"), true);
    assert.equal(JSON.parse(host.appInfoJson("dev.plinth.examples.counter")).blocked, true);
    host.unblock("dev.plinth.examples.counter");
    assert.throws(() => host.blockPublisher(""), /no publisher key/);
    host.blockPublisher(key);
    assert.equal(host.isPublisherBlocked(key), true);
    host.unblockPublisher(key);
    host.remove("dev.plinth.examples.counter");
    assert.throws(() => host.remove("dev.plinth.examples.counter"), /not in the library/);
    host.launch("dev.plinth.examples.notes");
    await waitFor(() => launches.length === 1, "launch");
    assert.deepEqual(launches, ["dev.plinth.examples.notes"]);

    // The state comes back from the store; package files are checked again.
    await host.flush();
    host = await new HubHost({ persist, onLaunch: (id) => launches.push(id) }).load();
    assert.deepEqual(JSON.parse(host.listAppsJson()).map((a) => a.id), ["dev.plinth.examples.notes", "dev.plinth.examples.utility"]);
    assert.deepEqual(JSON.parse(host.listGroupsJson()), ["Work", "Play"]);
    assert.ok((await host.packageBytes(host.runnableVersion("dev.plinth.examples.notes"))).length > 0);

    // Update: notes 0.2.0 adds clipboard.read (medium).
    const notes2 = path.join(dir, "notes2");
    cpSync(path.join(root, "examples/notes"), notes2, { recursive: true, filter: (p) => !p.includes(`${path.sep}dist`) && !p.includes(".plinth") });
    const toml = readFileSync(path.join(notes2, "plinth.toml"), "utf8").replace('version = "0.1.0"', 'version = "0.2.0"');
    writeFileSync(path.join(notes2, "plinth.toml"), `${toml}\n[[capabilities]]\nname = "clipboard.read"\nrationale = "Paste into a note."\n`);
    execFileSync(exe, ["build", notes2], { stdio: "ignore" });
    copyFileSync(path.join(notes2, "dist", "notes.plnt"), path.join(reg, "notes-0.2.0.plnt"));
    execFileSync(exe, ["registry", "build", reg, "--with-core"], { stdio: "ignore" });
    const check = JSON.parse(await host.checkUpdates(""));
    assert.deepEqual(check, {
      updates: [{ id: "dev.plinth.examples.notes", name: "Notes", current: "0.1.0", version: "0.2.0", source: "local", newCapabilities: ["clipboard.read"], pinned: "" }],
      errors: [],
    });
    assert.equal(JSON.parse(host.appInfoJson("dev.plinth.examples.notes")).update, "0.2.0");
    assert.equal(await host.update("dev.plinth.examples.notes"), "0.2.0");
    assert.equal(await host.update("dev.plinth.examples.notes"), "", "up to date");
    const after = JSON.parse(host.appInfoJson("dev.plinth.examples.notes"));
    assert.equal(after.update, "");
    assert.deepEqual(after.versions.map((v) => v.version), ["0.2.0", "0.1.0"]);
    // Until the user decides clipboard.read, the previous version runs.
    assert.equal(host.runnableVersion("dev.plinth.examples.notes").version, "0.1.0");
    const newest = host.newestInstalled("dev.plinth.examples.notes");
    assert.deepEqual(host.needsConsent("dev.plinth.examples.notes", newest.capabilities), ["clipboard.read"]);
    host.recordConsent("dev.plinth.examples.notes", newest.version, { "clipboard.read": true });
    assert.equal(host.runnableVersion("dev.plinth.examples.notes").version, "0.2.0");

    // A changed package in the registry is refused (digest).
    const counterDoc = JSON.parse(readFileSync(path.join(reg, "apps", "dev.plinth.examples.counter", "index.json"), "utf8"));
    const counterPkg = path.join(reg, "packages", `${counterDoc.versions[0].sha256}.plnt`);
    const bytes = readFileSync(counterPkg);
    bytes[bytes.length - 30] ^= 1;
    writeFileSync(counterPkg, bytes);
    await assert.rejects(host.install("dev.plinth.examples.counter"), /does not match the registry digest/);
    assert.equal(host.get("dev.plinth.examples.counter"), null);

    // The Hub app on this host.
    const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
    const hubPlnt = new Uint8Array(readFileSync(path.join(dir, "hub.plnt")));
    const hubUi = await runApp(coreBytes, hubPlnt, { hub: host });
    await waitFor(() => hubUi.allText().includes("Notes") && hubUi.allText().includes("Utility"), "the library lists the apps");
    await waitFor(() => /up to date|update/i.test(hubUi.allText()), "the update check at start");
    await hubUi.press("Notes");
    await hubUi.press("Open");
    await waitFor(() => launches.length === 2 && launches[1] === "dev.plinth.examples.notes", "Open asks the host to launch notes");

    // Without the backend, or with hub.manage refused: denied, no trap.
    const denied = await runApp(coreBytes, hubPlnt, {});
    await waitFor(() => /denied|unsupported/i.test(denied.allText()), "the Hub app says that the host refused");
    const refused = await runApp(coreBytes, hubPlnt, { hub: host, refused: new Set(["hub.manage"]) });
    await waitFor(() => /refused/i.test(refused.allText()), "hub.manage refused");

    console.log("run-hub-host.mjs: ok (server config, registry client, search/install/consent/groups/pins/blocks/updates, the Hub app on the web host)");
  } finally {
    server?.child.kill();
    rmSync(dir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
