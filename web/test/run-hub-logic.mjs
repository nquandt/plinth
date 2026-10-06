// Node test (docs/web-hub.md): builds a registry from the example apps with
// `plinth registry build --with-core`, serves it with
// `plinth registry serve --web`, and runs the web App Hub's logic
// (`hub-logic.js`, no DOM) against it: the app list, the capability labels
// from the registry, the browser support, search, and the URL that opens
// an app. Then it loads counter from that URL through the web host
// (`plinth-web.js`) and checks that its UI renders.
//
// Build first (scripts/ci-local.sh does this):
//   cargo build -p plinth-cli
//   target/debug/plinth build examples/<app>   # counter, notes, quotes, utility

import { spawn, execFileSync } from "node:child_process";
import { copyFileSync, mkdtempSync, rmSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import {
  loadRegistry,
  loadAppDocument,
  latestVersion,
  filterApps,
  capabilityRows,
  hasUnsupported,
  openTarget,
  pickCore,
  browserSupport,
} from "../hub-logic.js";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { riskOf } from "../hub-consent.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
const APPS = ["counter", "notes", "quotes", "utility"];

/** Starts `plinth registry serve <dir> --web --port 0` and returns the base URL from its output. */
function startServer(dir) {
  const child = spawn(exe, ["registry", "serve", dir, "--web", "--port", "0"], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
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

async function main() {
  const dir = mkdtempSync(path.join(tmpdir(), "plinth-web-hub-"));
  let server = null;
  try {
    for (const app of APPS) {
      const pkg = path.join(root, "examples", app, "dist", `${app}.plnt`);
      assert.ok(existsSync(pkg), `build ${app} first: ${pkg}`);
      copyFileSync(pkg, path.join(dir, `${app}.plnt`));
    }
    execFileSync(exe, ["registry", "build", dir, "--with-core"], { stdio: "ignore" });
    server = await startServer(dir);
    const { base } = server;

    // The landing page and the redirect.
    const landing = await fetch(base, { redirect: "manual" });
    assert.equal(landing.status, 302);
    assert.equal(landing.headers.get("location"), "/web/hub.html");
    const page = await fetch(new URL("/web/hub.html", base));
    assert.equal(page.headers.get("content-type"), "text/html; charset=utf-8");

    // The app list.
    const registry = await loadRegistry(base);
    assert.deepEqual(
      registry.apps.map((a) => a.name),
      ["Counter", "Notes", "Quotes", "Utility"],
    );
    assert.equal(registry.cores.length, 1, "--with-core gives one core");

    // Labels come from the registry (the shared capability map), not from this page.
    const notes = registry.apps.find((a) => a.name === "Notes");
    const [kv] = capabilityRows(notes);
    assert.equal(kv.name, "store.kv");
    assert.equal(kv.risk, "low");
    assert.equal(kv.description, "save data on this device");
    assert.ok(kv.rationale.length > 0, "the app's reason is in the label");
    assert.equal(kv.support, "yes");

    const quotes = registry.apps.find((a) => a.name === "Quotes");
    const [net] = capabilityRows(quotes);
    assert.equal(net.description, "connect to api.quotable.io");
    assert.equal(net.risk, "medium");
    assert.equal(net.support, "limited");

    const utility = registry.apps.find((a) => a.name === "Utility");
    const risks = Object.fromEntries(capabilityRows(utility).map((r) => [r.name, r.risk]));
    assert.deepEqual(risks, { "clipboard.write": "low", "clipboard.read": "medium" });

    // The consent logic has its own risk table (it must not trust the
    // registry); it agrees with the shared capability map.
    for (const app of registry.apps) {
      for (const label of app.labels ?? []) assert.equal(riskOf(label.name), label.risk, `${app.id}: ${label.name}`);
    }

    const counter = registry.apps.find((a) => a.name === "Counter");
    assert.deepEqual(capabilityRows(counter), []);
    assert.equal(hasUnsupported(capabilityRows(counter)), false);
    assert.equal(browserSupport("hub.manage").support, "no");
    assert.equal(hasUnsupported([{ name: "hub.manage", ...browserSupport("hub.manage") }]), true);

    // Search.
    assert.deepEqual(filterApps(registry.apps, "note").map((a) => a.name), ["Notes"]);
    assert.deepEqual(filterApps(registry.apps, "clipboard").map((a) => a.name), ["Utility"]);
    assert.deepEqual(filterApps(registry.apps, "plinth count").map((a) => a.name), ["Counter"]);
    assert.equal(filterApps(registry.apps, "").length, 4);
    assert.equal(filterApps(registry.apps, "zzz").length, 0);

    // Cores.
    assert.equal(pickCore([{ version: "1.2" }, { version: "1.9" }, { version: "2.0" }], "1.3").version, "1.9");
    assert.equal(pickCore([{ version: "1.2" }], "1.3"), null);

    // Open counter: the URL names the package and the core on this origin.
    const doc = await loadAppDocument(registry, counter.id);
    const version = latestVersion(doc);
    const target = openTarget(registry, version, `hub.html#/app/${counter.id}`);
    assert.ok(target.url.startsWith("index.html?"), target.url);
    const params = new URL(target.url, new URL("/web/hub.html", base)).searchParams;
    assert.equal(params.get("back"), `hub.html#/app/${counter.id}`);
    const pkgRes = await fetch(params.get("app"));
    const coreRes = await fetch(params.get("core"));
    assert.equal(coreRes.headers.get("content-type"), "application/wasm");
    const pltBytes = new Uint8Array(await pkgRes.arrayBuffer());
    const coreBytes = new Uint8Array(await coreRes.arrayBuffer());
    const { appWasm, manifestText } = await readPlnt(pltBytes);
    const app = new PlinthApp();
    const texts = [];
    app.onCommit = (ops) => {
      for (const op of ops) if (op.op === "text") texts.push(String(op.value));
    };
    await app.load(coreBytes, appWasm, { log: () => {}, manifestText });
    app.init([]);
    assert.ok(texts.some((t) => t.includes("0")), `counter renders its count: ${texts.join(" | ")}`);

    // No core: the page says why and offers no URL.
    const noCore = openTarget({ ...registry, cores: [] }, version);
    assert.equal(noCore.url, null);
    assert.match(noCore.reason, /--with-core/);

    console.log("run-hub-logic.mjs: ok (%d apps, core %s)", registry.apps.length, target.core);
  } finally {
    server?.child.kill();
    rmSync(dir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
