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

/** One request/response round trip over a CDP WebSocket connection (`sessionId`: an attached frame). */
function cdpSend(ws, pending, nextId, method, params = {}, sessionId = undefined) {
  const id = nextId.value++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
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
  // The web App Hub runs each app in a sandboxed iframe (an opaque origin),
  // which Edge puts in its own process: an "iframe" target. Auto-attach
  // gives a session for each such frame, to evaluate code in it.
  const frames = [];
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id !== undefined && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message));
      else resolve(msg.result);
    } else if (msg.method === "Target.attachedToTarget" && msg.params.targetInfo.type === "iframe") {
      frames.push(msg.params.sessionId);
    } else if (msg.method === "Target.detachedFromTarget") {
      const i = frames.indexOf(msg.params.sessionId);
      if (i >= 0) frames.splice(i, 1);
    }
  });
  const send = (method, params, sessionId) => cdpSend(ws, pending, nextId, method, params, sessionId);
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
  /** Evaluates `expression` in the newest app frame (undefined if there is none or it fails). */
  const frameEval = async (expression) => {
    const sessionId = frames.at(-1);
    if (!sessionId) return undefined;
    let res;
    try {
      res = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, sessionId);
    } catch {
      return undefined; // the frame is not ready yet, or it was replaced
    }
    if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text);
    return res.result?.value;
  };
  return {
    frameEval,
    /** Polls `expression` in the app frame until it is not undefined/null, or throws. */
    async frameWaitFor(expression, timeoutMs, intervalMs = 200) {
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        const v = await frameEval(expression).catch(() => undefined);
        if (v !== undefined && v !== null) return v;
        if (Date.now() > deadline) throw new Error(`timed out waiting in the app frame for: ${expression}`);
        await new Promise((r) => setTimeout(r, intervalMs));
      }
    },
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
 * Where an app runs: the page itself (`index.html`), or the sandboxed app
 * frame of the web App Hub (an "iframe" target; mouse positions add the
 * position of the frame).
 */
function pageSurface(page) {
  return { eval: (e) => page.eval(e), waitFor: (e, t) => page.waitFor(e, t), offset: async () => ({ x: 0, y: 0 }) };
}
function frameSurface(page) {
  return {
    eval: (e) => page.frameEval(e),
    waitFor: (e, t) => page.frameWaitFor(e, t),
    offset: () => page.eval(`(() => { const r = document.querySelector(".hub-frame").getBoundingClientRect(); return { x: r.x, y: r.y }; })()`),
  };
}

/** A real mouse press and release on the center of the element that `find` (an expression) gives. */
async function mouseClick(page, surface, find, holdMs = 0) {
  const r = await surface.eval(`(() => { const b = ${find}; if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
  if (!r) throw new Error(`nothing to click: ${find}`);
  const o = await surface.offset();
  const x = r.x + o.x;
  const y = r.y + o.y;
  await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
  await page.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button: "left", clickCount: 1 });
  if (holdMs) await new Promise((res) => setTimeout(res, holdMs));
  await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button: "left", clickCount: 1 });
}

/**
 * Clicks Stop on the running stopwatch with a real mouse press and release
 * 150 ms apart, while the timer re-renders the page (a regression: the
 * re-render replaced the button between press and release, so no click).
 * The app is already open in `surface`.
 */
async function stopwatchClicks(page, surface) {
  const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === ${JSON.stringify(label)})`;
  const shown = `document.querySelector("#app main").textContent`;
  await surface.waitFor(`${button("Start")} ? true : null`, 20000);
  await mouseClick(page, surface, button("Start"), 150);
  await new Promise((res) => setTimeout(res, 400));
  const before = await surface.eval(shown);
  await new Promise((res) => setTimeout(res, 200));
  if ((await surface.eval(shown)) === before) throw new Error("the stopwatch did not start");
  await mouseClick(page, surface, button("Stop"), 150);
  await new Promise((res) => setTimeout(res, 100));
  const stopped = await surface.eval(shown);
  await new Promise((res) => setTimeout(res, 400));
  const later = await surface.eval(shown);
  if (later !== stopped) throw new Error(`one Stop click did not stop the stopwatch: "${stopped}" then "${later}"`);
}

async function checkStopwatch(cdpPort, base) {
  const appUrl = `${base}/examples/timer/dist/timer.plnt`;
  const coreUrl = `${base}/target/core.wasm`;
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`);
    await stopwatchClicks(page, pageSurface(page));
  } finally {
    await page.close();
  }
}

/** Types into todo's TextField key by key, as a person does (real input events). The app is already open. */
async function typeKeys(page, surface, focus) {
  await surface.waitFor(`document.querySelector("#app input[type=text]") ? true : null`, 20000);
  await focus();
  const word = "milk";
  for (let i = 0; i < word.length; i++) {
    await page.send("Input.insertText", { text: word[i] });
    const want = word.slice(0, i + 1);
    const state = await surface
      .waitFor(
        `(() => { const a = document.activeElement; return a && a.tagName === "INPUT" && a.value === ${JSON.stringify(want)} ? { caret: a.selectionStart } : null; })()`,
        3000,
      )
      .catch(async () => {
        const got = await surface.eval(`({ tag: document.activeElement?.tagName, value: document.querySelector("#app input[type=text]")?.value })`);
        throw new Error(`after typing "${want}": focus on ${got?.tag}, field value "${got?.value}"`);
      });
    if (state.caret !== want.length) throw new Error(`after typing "${want}": caret at ${state.caret}, not ${want.length}`);
  }
}

async function checkTyping(cdpPort, base) {
  const appUrl = `${base}/examples/todo/dist/todo.plnt`;
  const coreUrl = `${base}/target/core.wasm`;
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`);
    const surface = pageSurface(page);
    await typeKeys(page, surface, () => surface.eval(`document.querySelector("#app input[type=text]").focus()`));
  } finally {
    await page.close();
  }
}

const AXE_URL = "https://cdnjs.cloudflare.com/ajax/libs/axe-core/4.9.1/axe.min.js";

/**
 * Runs axe-core in the page or in the app frame (loads it from the CDN the
 * first time). The hub views skip iframes: the app frame has an opaque
 * origin, so axe checks it on its own (`evaluate` = the frame's evaluator).
 */
async function axeRun(page, evaluate = (e) => page.eval(e), options = {}) {
  const json = await evaluate(`(async () => {
    if (!window.axe) {
      await new Promise((resolve, reject) => {
        const s = document.createElement("script");
        s.src = ${JSON.stringify(AXE_URL)};
        s.onload = resolve;
        s.onerror = () => reject(new Error("cannot load axe-core"));
        document.head.append(s);
      });
    }
    const r = await axe.run(document, ${JSON.stringify(options)});
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

const HUB_APPS = ["counter", "notes", "budget", "quotes", "utility", "todo", "timer", "hub"];
const ID = (app) => (app === "hub" ? "dev.plinth.hub" : `dev.plinth.examples.${app}`);

/**
 * The web App Hub in headless Edge: the list renders (light and dark, and
 * at phone width with no horizontal scroll), app pages show the capability
 * label and the browser support, and apps run in the sandboxed app frame:
 * counter with no question; notes and budget after the consent view, each
 * with its own kv data that the other cannot see; the Manage page changes a
 * grant and removes data; a package that does not match the registry digest
 * does not run; typing in todo and the stopwatch of timer work in the frame.
 * Returns the axe result of each view.
 */
async function checkHub(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-hub-"));
  for (const app of HUB_APPS) {
    await copyFile(path.join(root, "examples", app, "dist", `${app}.plnt`), path.join(dir, `${app}.plnt`));
  }
  execFileSync(exe, ["registry", "build", dir, "--with-core"], { stdio: "ignore" });
  // Change one byte of the utility package after the build: the registry
  // digest no longer agrees, so the hub must refuse to run it.
  const utilityDoc = JSON.parse(await readFile(path.join(dir, "apps", ID("utility"), "index.json"), "utf8"));
  const utilityPkg = path.join(dir, "packages", `${utilityDoc.versions[0].sha256}.plnt`);
  const bytes = await readFile(utilityPkg);
  bytes[bytes.length - 30] ^= 1;
  await writeFile(utilityPkg, bytes);

  const { child, base } = await startHubServer(exe, dir);
  const results = [];
  const page = await openPage(cdpPort);
  const frame = frameSurface(page);
  try {
    const ready = `document.body.dataset.ready === "true" ? true : null`;
    // A11Y_SHOTS=<folder>: also save a PNG of each view, to look at by eye.
    const shot = async (name) => {
      if (!process.env.A11Y_SHOTS) return;
      await mkdir(process.env.A11Y_SHOTS, { recursive: true });
      const r = await page.send("Page.captureScreenshot", { format: "png", captureBeyondViewport: true });
      await writeFile(path.join(process.env.A11Y_SHOTS, `hub-${name}.png`), Buffer.from(r.data, "base64"));
    };
    const hubAxe = async (view) => results.push({ view, violations: await axeRun(page, undefined, { iframes: false }) });
    const go = async (hash, until) => {
      await page.eval(`location.hash = ${JSON.stringify(hash)}`);
      await page.waitFor(`document.body.dataset.route === ${JSON.stringify(hash)} ${until ? `&& (${until})` : ""} ? true : null`, 20000);
    };
    const running = (id) => `document.body.dataset.running === ${JSON.stringify(id)}`;
    const mainText = () => page.eval(`document.getElementById("hub-main").innerText`);
    const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === ${JSON.stringify(label)})`;

    for (const scheme of ["light", "dark"]) {
      await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
      await page.navigate(`${base}/`);
      await page.waitFor(ready, 20000);
      const cards = await page.eval(`document.querySelectorAll(".hub-card").length`);
      if (cards !== HUB_APPS.length) throw new Error(`the list shows ${cards} apps, not ${HUB_APPS.length}`);
      await hubAxe(`list (${scheme})`);
      await shot(`list-${scheme}`);
    }
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });

    // Search.
    await page.eval(`(() => { const q = document.getElementById("hub-q"); q.value = "clip"; q.dispatchEvent(new Event("input")); })()`);
    const found = await page.eval(`[...document.querySelectorAll(".hub-card h2")].map((h) => h.textContent).join(",")`);
    if (found !== "Utility") throw new Error(`search "clip" shows ${found}`);

    // App pages: the label, the risk levels and the browser support.
    const appPage = async (id) => {
      await go(`#/app/${id}`);
      return mainText();
    };
    const utility = await appPage(ID("utility"));
    for (const want of ["Read the clipboard.", "Medium risk", "Write to the clipboard.", "Low risk", "Limited in the browser"]) {
      if (!utility.includes(want)) throw new Error(`the Utility page has no "${want}":\n${utility}`);
    }
    await hubAxe("app page (utility)");
    await shot("app-utility");
    const hubApp = await appPage(ID("hub"));
    for (const want of ["High risk", "Not supported in the browser", "does not support hub.manage"]) {
      if (!hubApp.includes(want)) throw new Error(`the Hub page has no "${want}":\n${hubApp}`);
    }
    await shot("app-hub");

    // Open counter (no capabilities): no question, it runs in the sandboxed frame.
    await appPage(ID("counter"));
    await page.eval(`document.querySelector(".hub-open").click()`);
    await page.waitFor(`${running(ID("counter"))} ? true : null`, 20000);
    const sandbox = await page.eval(`document.querySelector(".hub-frame").getAttribute("sandbox")`);
    if (sandbox !== "allow-scripts") throw new Error(`the app frame has sandbox="${sandbox}"`);
    await frame.waitFor(`document.querySelector("#app button") ? true : null`, 20000);
    // The frame has an opaque origin: no storage, no cookies, no hub DOM.
    const walls = await frame.eval(`(() => {
      const tryIt = (f) => { try { f(); return "open"; } catch { return "blocked"; } };
      return { origin: self.origin, storage: tryIt(() => localStorage.length), cookie: tryIt(() => document.cookie),
               parent: tryIt(() => parent.document.body), idb: typeof indexedDB === "undefined" ? "blocked" : tryIt(() => indexedDB.open("x")) };
    })()`);
    if (walls.origin !== "null" || walls.storage !== "blocked" || walls.cookie !== "blocked" || walls.parent !== "blocked") {
      throw new Error(`the app frame is not isolated: ${JSON.stringify(walls)}`);
    }
    await frame.eval(`${button("Increment")}.click()`);
    await frame.waitFor(`[...document.querySelectorAll("#app h1, #app h2")].some((h) => h.textContent.trim() === "1") ? true : null`, 5000);
    await hubAxe("counter running (hub page)");
    results.push({ view: "counter in the app frame", violations: await axeRun(page, frame.eval) });
    await shot("counter");

    // Notes: the consent view comes first (store.kv is low risk: Allow is selected).
    await go(`#/run/${ID("notes")}`, `document.body.dataset.consent`);
    const consent = await mainText();
    for (const want of ["Allow Notes to use these capabilities?", "Save data on this device.", "Low risk", "Save your notes on this device.", "Not signed"]) {
      if (!consent.includes(want)) throw new Error(`the consent view has no "${want}":\n${consent}`);
    }
    const checked = await page.eval(`document.querySelector("input[data-cap='store.kv']:checked")?.value`);
    if (checked !== "allow") throw new Error(`store.kv (low risk) is not "allow" by default: ${checked}`);
    await hubAxe("consent (notes)");
    await shot("consent-notes");
    await page.eval(`document.getElementById("hub-consent-ok").click()`);
    await page.waitFor(`${running(ID("notes"))} ? true : null`, 20000);
    await frame.waitFor(`document.body.dataset.kvEntries === "0" ? true : null`, 5000);
    const addNote = async (title) => {
      await frame.eval(`(() => {
        const set = (label, v) => { const i = [...document.querySelectorAll("#app input, #app textarea")].find((e) => e.labels?.[0]?.textContent.trim() === label || e.getAttribute("aria-label") === label);
          i.value = v; i.dispatchEvent(new Event("input", { bubbles: true })); };
        set("Title", ${JSON.stringify(title)}); set("Body", "From the a11y test");
      })()`);
      await frame.eval(`${button("Add note")}.click()`);
      await frame.waitFor(`document.getElementById("app").innerText.includes(${JSON.stringify(title)}) ? true : null`, 5000);
    };
    await addNote("Isolated note");
    const keysOf = (id) => `Object.keys(localStorage).filter((k) => k.startsWith("plinth-hub:kv:${id}:"))`;
    await page.waitFor(`${keysOf(ID("notes"))}.length > 0 ? true : null`, 5000);

    // Budget: its own consent, and its frame gets none of the notes data.
    const budgetBefore = await page.eval(`${keysOf(ID("budget"))}.length`);
    await go(`#/run/${ID("budget")}`, `document.body.dataset.consent`);
    await page.eval(`document.getElementById("hub-consent-ok").click()`);
    await page.waitFor(`${running(ID("budget"))} ? true : null`, 20000);
    const budgetGot = await frame.waitFor(`document.body.dataset.kvEntries ?? null`, 5000);
    if (Number(budgetGot) !== budgetBefore) throw new Error(`budget got ${budgetGot} kv entries, not ${budgetBefore} (the notes data must not be there)`);
    const budgetKeys = await page.eval(`${keysOf(ID("budget"))}.length`);
    if ((await frame.eval(`document.getElementById("app").innerText`)).includes("Isolated note")) throw new Error("budget shows the note of notes");

    // Notes again: no question, the note is still there (from the parent's snapshot).
    await go(`#/run/${ID("notes")}`, running(ID("notes")));
    await frame.waitFor(`document.getElementById("app").innerText.includes("Isolated note") ? true : null`, 10000);

    // Manage: deny store.kv for notes; notes runs and reads nothing.
    await go("#/manage");
    const manage = await mainText();
    for (const want of ["Manage apps", "Notes", "Budget", "Allowed (default)", "Stored data"]) {
      if (!manage.includes(want)) throw new Error(`the Manage page has no "${want}":\n${manage}`);
    }
    await hubAxe("manage");
    await shot("manage");
    const notesSection = `document.querySelector('section[data-app="${ID("notes")}"]')`;
    await page.eval(`(() => { const s = ${notesSection}.querySelector("select[data-cap='store.kv']"); s.value = "deny"; s.dispatchEvent(new Event("change")); })()`);
    await page.waitFor(`${notesSection}.innerText.includes("Denied") ? true : null`, 5000);
    await go(`#/run/${ID("notes")}`, running(ID("notes")));
    await frame.waitFor(`document.querySelector("#app button") ? true : null`, 10000);
    if ((await frame.eval(`document.getElementById("app").innerText`)).includes("Isolated note")) throw new Error("notes reads its data with store.kv denied");

    // Manage: remove the data of notes, forget the decisions: the next run asks again.
    await go("#/manage");
    await page.eval(`[...${notesSection}.querySelectorAll("button")].find((b) => b.textContent === "Remove stored data").click()`);
    await page.waitFor(`${keysOf(ID("notes"))}.length === 0 ? true : null`, 5000);
    if ((await page.eval(`${keysOf(ID("budget"))}.length`)) !== budgetKeys) throw new Error("removing the notes data changed budget");
    await page.eval(`[...${notesSection}.querySelectorAll("button")].find((b) => b.textContent === "Forget decisions").click()`);
    await go(`#/run/${ID("notes")}`, `document.body.dataset.consent`);

    // A package that does not match the registry digest does not run.
    await go(`#/run/${ID("utility")}`, `document.body.dataset.refused`);
    const refused = await mainText();
    if (!refused.includes("does not match the registry digest")) throw new Error(`no digest message:\n${refused}`);
    if (await page.eval(`document.querySelector(".hub-frame") !== null`)) throw new Error("a refused package has an app frame");
    await hubAxe("refused package");

    // Typing and the stopwatch inside the app frame.
    await go(`#/run/${ID("todo")}`, running(ID("todo")));
    await typeKeys(page, frame, async () => {
      // A real click first. Headless Edge does not always move the focus
      // into a new out-of-process frame on a synthetic click (about 1 run
      // in 6: the iframe element gets the focus, the frame document does
      // not); then focus the field from inside the frame, as the
      // standalone check does. The keys then go to the field.
      const input = `document.querySelector("#app input[type=text]")`;
      await mouseClick(page, frame, input);
      const focused = `document.activeElement?.tagName === "INPUT" && document.hasFocus() ? true : null`;
      if (!(await frame.waitFor(focused, 1500).catch(() => false))) {
        await frame.eval(`(window.focus(), ${input}.focus())`);
        await frame.waitFor(focused, 2000);
      }
    });
    await go(`#/run/${ID("timer")}`, running(ID("timer")));
    await stopwatchClicks(page, frame);
    // Phone width: no horizontal scroll. Last, because the device metrics
    // override does not reach the out-of-process app frames in the same way
    // (their clicks then miss).
    await page.send("Emulation.setDeviceMetricsOverride", { width: 390, height: 800, deviceScaleFactor: 2, mobile: true });
    await appPage(ID("quotes"));
    const overflow = await page.eval(`document.documentElement.scrollWidth - window.innerWidth`);
    if (overflow > 0) throw new Error(`the app page scrolls sideways by ${overflow}px at 390px`);
    await hubAxe("app page (quotes, phone)");
    await shot("app-quotes-phone");
    await go("#/");
    const listOverflow = await page.eval(`document.documentElement.scrollWidth - window.innerWidth`);
    if (listOverflow > 0) throw new Error(`the list scrolls sideways by ${listOverflow}px at 390px`);
    await shot("list-phone");
    await page.send("Emulation.clearDeviceMetricsOverride");

    console.log("  list, search, app pages, phone width, sandboxed frame, consent, kv isolation, Manage, digest check, typing and stopwatch in the frame: ok");
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
