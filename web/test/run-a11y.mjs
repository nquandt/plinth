// Automated accessibility check (SPEC.md §6): loads `web/test/a11y.html` for
// each example app in headless Edge, runs axe-core, and fails on any
// serious/critical violation.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm
//   target/debug/plinth.exe build examples/<app>   # for each app below
//
// Needs network access to cdnjs.cloudflare.com (axe-core is loaded from
// there by a11y.html, not vendored).
//
// Driven over the Chrome DevTools Protocol (plain `WebSocket`/`fetch`,
// Node 22 has both built in -- no puppeteer dependency) rather than
// `--dump-dom --virtual-time-budget`: that combination stalls Chromium's
// virtual clock on real WebAssembly compilation (observed: `app.load()`
// alone took ~15s under virtual time, vs. a few ms normally), making the
// dump race the page's own async work. CDP lets us just poll the real page
// in real time until `#result` is populated.

import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { readFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

const APPS = ["counter", "todo", "settings-gallery", "contacts", "budget"];

const EDGE = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";

const MIME = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".mjs": "text/javascript",
  ".css": "text/css",
  ".wasm": "application/wasm",
  ".plnt": "application/zip",
  ".json": "application/json",
};

/** A tiny static file server rooted at the repo root (no dependency). */
function startServer() {
  return new Promise((resolve, reject) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        const filePath = path.join(root, urlPath);
        if (!filePath.startsWith(root)) {
          res.writeHead(403);
          res.end();
          return;
        }
        const data = await readFile(filePath);
        const ext = path.extname(filePath);
        res.writeHead(200, { "Content-Type": MIME[ext] ?? "application/octet-stream" });
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

/** Waits for Edge's `/json/version` endpoint, so we know CDP is ready. */
async function waitForCdp(port, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (r.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error("headless Edge did not open its DevTools port in time");
}

/** One request/response round trip over a CDP WebSocket connection. */
function cdpSend(ws, pending, nextId, method, params = {}) {
  const id = nextId.value++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
}

/** Opens a new page in the given browser-level CDP connection and returns a
 * helper bound to that page's target, plus a `close()` to tear it down. */
async function openPage(cdpPort) {
  const r = await fetch(`http://127.0.0.1:${cdpPort}/json/new?about:blank`, { method: "PUT" });
  const target = await r.json();
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  const pending = new Map();
  const nextId = { value: 1 };
  await new Promise((resolve, reject) => {
    ws.addEventListener("open", resolve, { once: true });
    ws.addEventListener("error", reject, { once: true });
  });
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id !== undefined && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message));
      else resolve(msg.result);
    }
  });
  const send = (method, params) => cdpSend(ws, pending, nextId, method, params);
  await send("Page.enable");
  await send("Runtime.enable");
  return {
    async navigate(url) {
      await send("Page.navigate", { url });
    },
    /** Polls `expression` (must return a JSON-serializable value or
     * undefined) in the page until it is not undefined/null, or throws. */
    async waitFor(expression, timeoutMs, intervalMs = 200) {
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        const res = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: false });
        const v = res.result?.value;
        if (v !== undefined && v !== null) return v;
        if (Date.now() > deadline) throw new Error(`timed out waiting for: ${expression}`);
        await new Promise((r) => setTimeout(r, intervalMs));
      }
    },
    async close() {
      ws.close();
      await fetch(`http://127.0.0.1:${cdpPort}/json/close/${target.id}`).catch(() => {});
    },
  };
}

async function main() {
  const server = await startServer();
  const port = server.address().port;
  const base = `http://127.0.0.1:${port}`;

  const cdpPort = 19 * 1000 + Math.floor(Math.random() * 1000) + 222; // avoid colliding across parallel runs
  const userDataDir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-"));
  const edge = spawn(
    EDGE,
    [
      "--headless=new",
      "--disable-gpu",
      "--no-sandbox",
      `--remote-debugging-port=${cdpPort}`,
      `--user-data-dir=${userDataDir}`,
      "about:blank",
    ],
    { windowsHide: true, stdio: "ignore" }
  );

  let failed = false;
  const summary = [];

  try {
    await waitForCdp(cdpPort);

    for (const app of APPS) {
      const appUrl = `${base}/examples/${app}/dist/${app}.plnt`;
      const coreUrl = `${base}/target/core.wasm`;
      const testUrl = `${base}/web/test/a11y.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`;
      console.log(`== ${app} ==`);

      let result = null;
      let lastError = null;
      const ATTEMPTS = 2;
      for (let attempt = 1; attempt <= ATTEMPTS && result === null; attempt++) {
        const page = await openPage(cdpPort);
        try {
          await page.navigate(testUrl);
          const value = await page.waitFor(
            `(() => { const el = document.getElementById("result"); return el && el.style.display !== "none" ? el.textContent : null; })()`,
            20000
          );
          const parsed = JSON.parse(value);
          if (parsed.error) lastError = parsed.error;
          else result = parsed;
        } catch (err) {
          lastError = err.message;
        } finally {
          await page.close();
        }
      }

      if (result === null) {
        console.error(`  FAILED after ${ATTEMPTS} attempts: ${lastError}`);
        failed = true;
        continue;
      }

      const violations = result.violations ?? [];
      const serious = violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      const minor = violations.filter((v) => v.impact !== "serious" && v.impact !== "critical");
      summary.push({ app, serious: serious.length, minor: minor.length });
      for (const v of violations) {
        console.log(`  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))`);
        if (process.env.A11Y_VERBOSE) {
          for (const n of v.nodes) console.log(`      ${n.target}\n      ${n.html}\n      ${n.failureSummary}`);
        }
      }
      if (serious.length === 0 && minor.length === 0) console.log("  no violations");
      if (serious.length > 0) failed = true;
    }
  } finally {
    edge.kill();
    server.close();
    await rm(userDataDir, { recursive: true, force: true }).catch(() => {});
  }

  console.log("\n== summary ==");
  for (const s of summary) console.log(`  ${s.app}: serious/critical=${s.serious} minor=${s.minor}`);

  if (failed) {
    console.error("\nFAILED: serious/critical accessibility violations found (or a run failed).");
    process.exit(1);
  }
  console.log("\nOK: no serious/critical accessibility violations.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
