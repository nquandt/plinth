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

import { spawn, execFileSync } from "node:child_process";
import { createServer } from "node:http";
import { readFile, mkdtemp, rm, copyFile, writeFile, mkdir } from "node:fs/promises";
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
    /** Evaluates `expression` and awaits it if it is a promise. */
    async eval(expression) {
      const res = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true });
      if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text);
      return res.result?.value;
    },
    send,
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
  const hubOnly = process.argv.includes("--hub-only");

  try {
    await waitForCdp(cdpPort);

    for (const app of hubOnly || process.argv.includes("--typing-only") ? [] : APPS) {
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

    // Typing: each key re-renders the page, and the TextField must keep the
    // focus and the caret (a regression: the first key moved the focus away).
    console.log("== typing in todo ==");
    try {
      await checkTyping(cdpPort, base);
      console.log("  focus and caret kept after each key: ok");
    } catch (err) {
      console.error(`  FAILED: ${err.stack ?? err}`);
      failed = true;
    }
    console.log("== one click stops the stopwatch ==");
    try {
      await checkStopwatch(cdpPort, base);
      console.log("  ok");
    } catch (err) {
      console.error(`  FAILED: ${err.stack ?? err}`);
      failed = true;
    }
    // The web App Hub (docs/web-hub.md): served by `plinth registry serve --web`.
    if (!process.argv.includes("--typing-only")) console.log("== web App Hub ==");
    if (!process.argv.includes("--typing-only")) try {
      const hub = await checkHub(cdpPort);
      for (const r of hub) {
        const serious = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
        summary.push({ app: `hub: ${r.view}`, serious: serious.length, minor: r.violations.length - serious.length });
        for (const v of r.violations) {
          console.log(`  ${r.view}: [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))`);
          if (process.env.A11Y_VERBOSE) for (const n of v.nodes) console.log(`      ${n.target}\n      ${n.html}\n      ${n.failureSummary}`);
        }
        if (serious.length > 0) failed = true;
      }
    } catch (err) {
      console.error(`  FAILED: ${err.stack ?? err}`);
      failed = true;
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

/**
 * Clicks Stop on the running stopwatch with a real mouse press and release
 * 150 ms apart, while the timer re-renders the page (a regression: the
 * re-render replaced the button between press and release, so no click).
 */
async function checkStopwatch(cdpPort, base) {
  const appUrl = `${base}/examples/timer/dist/timer.plnt`;
  const coreUrl = `${base}/target/core.wasm`;
  const page = await openPage(cdpPort);
  const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === ${JSON.stringify(label)})`;
  const shown = `document.querySelector("#app main").textContent`;
  const click = async (label) => {
    const r = await page.eval(`(() => { const b = ${button(label)}; if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    if (!r) throw new Error(`no "${label}" button`);
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: r.x, y: r.y });
    await page.send("Input.dispatchMouseEvent", { type: "mousePressed", x: r.x, y: r.y, button: "left", clickCount: 1 });
    await new Promise((res) => setTimeout(res, 150));
    await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: r.x, y: r.y, button: "left", clickCount: 1 });
  };
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`);
    await page.waitFor(`${button("Start")} ? true : null`, 20000);
    await click("Start");
    await new Promise((res) => setTimeout(res, 400));
    const before = await page.eval(shown);
    await new Promise((res) => setTimeout(res, 200));
    if ((await page.eval(shown)) === before) throw new Error("the stopwatch did not start");
    await click("Stop");
    await new Promise((res) => setTimeout(res, 100));
    const stopped = await page.eval(shown);
    await new Promise((res) => setTimeout(res, 400));
    const later = await page.eval(shown);
    if (later !== stopped) throw new Error(`one Stop click did not stop the stopwatch: "${stopped}" then "${later}"`);
  } finally {
    await page.close();
  }
}

/** Types into todo's TextField key by key, as a person does (real input events). */
async function checkTyping(cdpPort, base) {
  const appUrl = `${base}/examples/todo/dist/todo.plnt`;
  const coreUrl = `${base}/target/core.wasm`;
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`);
    await page.waitFor(`document.querySelector("#app input[type=text]") ? true : null`, 20000);
    await page.eval(`document.querySelector("#app input[type=text]").focus()`);
    const word = "milk";
    for (let i = 0; i < word.length; i++) {
      await page.send("Input.insertText", { text: word[i] });
      const want = word.slice(0, i + 1);
      const state = await page.waitFor(
        `(() => { const a = document.activeElement; return a && a.tagName === "INPUT" && a.value === ${JSON.stringify(want)} ? { caret: a.selectionStart } : null; })()`,
        3000
      ).catch(async () => {
        const got = await page.eval(`({ tag: document.activeElement?.tagName, value: document.querySelector("#app input[type=text]")?.value })`);
        throw new Error(`after typing "${want}": focus on ${got.tag}, field value "${got.value}"`);
      });
      if (state.caret !== want.length) throw new Error(`after typing "${want}": caret at ${state.caret}, not ${want.length}`);
    }
  } finally {
    await page.close();
  }
}

const AXE_URL ="https://cdnjs.cloudflare.com/ajax/libs/axe-core/4.9.1/axe.min.js";

/** Runs axe-core in the page (loads it from the CDN the first time). */
async function axeRun(page) {
  const json = await page.eval(`(async () => {
    if (!window.axe) {
      await new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = ${JSON.stringify(AXE_URL)};
        s.onload = resolve;
        s.onerror = () => reject(new Error("cannot load axe-core"));
        document.head.append(s);
      });
    }
    const r = await axe.run(document);
    return JSON.stringify(r.violations.map((v) => ({ id: v.id, impact: v.impact, help: v.help,
      nodes: v.nodes.map((n) => ({ target: n.target, html: n.html, failureSummary: n.failureSummary })) })));
  })()`);
  return JSON.parse(json);
}

/** Starts `plinth registry serve <dir> --web --port 0`; resolves with the base URL. */
function startHubServer(exe, dir) {
  const child = spawn(exe, ["registry", "serve", dir, "--web", "--port", "0"], { stdio: ["ignore", "pipe", "inherit"], windowsHide: true });
  return new Promise((resolve, reject) => {
    let out = "";
    child.stdout.on("data", (chunk) => {
      out += chunk;
      const m = /on (http:\/\/127\.0\.0\.1:\d+)/.exec(out);
      if (m) resolve({ child, base: m[1] });
    });
    child.on("exit", (code) => reject(new Error(`the hub server stopped (${code}): ${out}`)));
  });
}

/**
 * The web App Hub in headless Edge: the list renders (light and dark, and
 * at phone width with no horizontal scroll), app pages show the capability
 * label and the browser support, and Open runs counter in the web host.
 * Returns the axe result of each view.
 */
async function checkHub(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-hub-"));
  for (const app of ["counter", "notes", "quotes", "utility", "hub"]) {
    await copyFile(path.join(root, "examples", app, "dist", `${app}.plnt`), path.join(dir, `${app}.plnt`));
  }
  execFileSync(exe, ["registry", "build", dir, "--with-core"], { stdio: "ignore" });
  const { child, base } = await startHubServer(exe, dir);
  const results = [];
  const page = await openPage(cdpPort);
  try {
    const ready = `document.body.dataset.ready === "true" ? true : null`;
    // A11Y_SHOTS=<folder>: also save a PNG of each view, to look at by eye.
    const shot = async (name) => {
      if (!process.env.A11Y_SHOTS) return;
      await mkdir(process.env.A11Y_SHOTS, { recursive: true });
      const r = await page.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
      await writeFile(path.join(process.env.A11Y_SHOTS, `hub-${name}.png`), Buffer.from(r.data, "base64"));
    };
    for (const scheme of ["light", "dark"]) {
      await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
      await page.navigate(`${base}/`);
      await page.waitFor(ready, 20000);
      const cards = await page.eval(`document.querySelectorAll(".hub-card").length`);
      if (cards !== 5) throw new Error(`the list shows ${cards} apps, not 5`);
      results.push({ view: `list (${scheme})`, violations: await axeRun(page) });
      await shot(`list-${scheme}`);
    }

    // Search.
    await page.eval(`(() => { const q = document.getElementById("hub-q"); q.value = "clip"; q.dispatchEvent(new Event("input")); })()`);
    const found = await page.eval(`[...document.querySelectorAll(".hub-card h2")].map((h) => h.textContent).join(",")`);
    if (found !== "Utility") throw new Error(`search "clip" shows ${found}`);

    // App pages: the label, the risk levels and the browser support.
    const appPage = async (id) => {
      await page.eval(`location.hash = "#/app/${id}"`);
      await page.waitFor(`document.body.dataset.route === "#/app/${id}" ? true : null`, 10000);
      return page.eval(`document.getElementById("hub-main").innerText`);
    };
    const utility = await appPage("dev.plinth.examples.utility");
    for (const want of ["Read the clipboard.", "Medium risk", "Write to the clipboard.", "Low risk", "Limited in the browser"]) {
      if (!utility.includes(want)) throw new Error(`the Utility page has no "${want}":\n${utility}`);
    }
    results.push({ view: "app page (utility)", violations: await axeRun(page) });
    await shot("app-utility");
    const hubApp = await appPage("dev.plinth.hub");
    for (const want of ["High risk", "Not supported in the browser", "does not support hub.manage"]) {
      if (!hubApp.includes(want)) throw new Error(`the Hub page has no "${want}":\n${hubApp}`);
    }

    await shot("app-hub");

    // Phone width: no horizontal scroll.
    await page.send("Emulation.setDeviceMetricsOverride", { width: 390, height: 800, deviceScaleFactor: 2, mobile: true });
    await appPage("dev.plinth.examples.quotes");
    const overflow = await page.eval(`document.documentElement.scrollWidth - window.innerWidth`);
    if (overflow > 0) throw new Error(`the app page scrolls sideways by ${overflow}px at 390px`);
    results.push({ view: "app page (quotes, phone)", violations: await axeRun(page) });
    await shot("app-quotes-phone");
    await page.eval(`location.hash = "#/"`);
    await page.waitFor(`document.querySelector(".hub-card") ? true : null`, 10000);
    const listOverflow = await page.eval(`document.documentElement.scrollWidth - window.innerWidth`);
    if (listOverflow > 0) throw new Error(`the list scrolls sideways by ${listOverflow}px at 390px`);
    await shot("list-phone");
    await page.send("Emulation.clearDeviceMetricsOverride");

    // Open counter: the web host renders its UI, with a link back.
    await appPage("dev.plinth.examples.counter");
    await page.eval(`document.querySelector(".hub-open").click()`);
    await page.waitFor(`document.querySelector("#app button") && !document.getElementById("host-bar").hidden ? true : null`, 20000);
    const text = await page.eval(`document.getElementById("app").innerText`);
    if (!/0/.test(text)) throw new Error(`counter did not render its count: ${text}`);
    await page.eval(`[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === "Increment").click()`);
    await page.waitFor(`[...document.querySelectorAll("#app h1, #app h2")].some((h) => h.textContent.trim() === "1") ? true : null`, 5000);
    results.push({ view: "counter opened from the hub", violations: await axeRun(page) });
    await shot("counter");
    console.log("  list, search, app pages, phone width and Open counter: ok");
  } finally {
    await page.close();
    child.kill();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
  return results;
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
