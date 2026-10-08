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
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

const APPS = ["counter", "todo", "settings-gallery", "contacts", "budget", "primitives"];

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

/** A tiny static file server rooted at `dir` (the repo root by default; no dependency, no CORS headers). */
function startServer(dir = root) {
  return new Promise((resolve, reject) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        const filePath = path.join(dir, urlPath.endsWith("/") ? `${urlPath}index.html` : urlPath);
        if (!filePath.startsWith(dir)) {
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

/**
 * Closes Edge through CDP `Browser.close` (it closes the tabs first), then
 * kills the process if it is still there. A plain kill leaves the tabs
 * registered with the Windows shell, so they stay in Alt+Tab as ghosts.
 */
async function closeEdge(cdpPort, edge) {
  try {
    const version = await (await fetch(`http://127.0.0.1:${cdpPort}/json/version`)).json();
    const ws = new WebSocket(version.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    const exited = new Promise((resolve) => edge.once("exit", resolve));
    ws.send(JSON.stringify({ id: 1, method: "Browser.close" }));
    await Promise.race([exited, new Promise((r) => setTimeout(r, 5000))]);
  } catch {
    // Edge is gone already, or CDP did not answer: kill it below.
  }
  if (edge.exitCode === null) edge.kill();
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
    // No call waits for ever: a stuck call (an evaluate whose promise never
    // settles, a dead frame) fails with its name instead of hanging the run.
    const timer = setTimeout(() => {
      if (!pending.delete(id)) return;
      const what = method === "Runtime.evaluate" ? `${method} ${String(params.expression).slice(0, 120)}` : method;
      reject(new Error(`no answer in ${CDP_TIMEOUT_MS / 1000} s to ${what}`));
    }, CDP_TIMEOUT_MS);
    pending.set(id, {
      resolve: (v) => (clearTimeout(timer), resolve(v)),
      reject: (e) => (clearTimeout(timer), reject(e)),
    });
    ws.send(JSON.stringify({ id, method, params, sessionId }));
  });
}

/** The longest time that one DevTools call may take. */
const CDP_TIMEOUT_MS = 60000;

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
  // A sandboxed frame can also stay in the page process (a `srcdoc` frame
  // of a web export can): then its default execution context is the way in.
  const childContexts = [];
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id !== undefined && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(msg.error.message));
      else resolve(msg.result);
    } else if (msg.method === "Target.attachedToTarget" && msg.params.targetInfo.type === "iframe") {
      frames.push(msg.params.sessionId);
    } else if (msg.method === "Runtime.executionContextCreated" && !msg.sessionId) {
      const ctx = msg.params.context;
      if (ctx.auxData?.isDefault && ctx.auxData.frameId !== target.id) childContexts.push(ctx.id);
    } else if (msg.method === "Runtime.executionContextDestroyed" && !msg.sessionId) {
      const i = childContexts.indexOf(msg.params.executionContextId);
      if (i >= 0) childContexts.splice(i, 1);
    } else if (msg.method === "Runtime.executionContextsCleared" && !msg.sessionId) {
      childContexts.length = 0;
    } else if (msg.method === "Target.detachedFromTarget") {
      const i = frames.indexOf(msg.params.sessionId);
      if (i >= 0) frames.splice(i, 1);
    }
  });
  const send = (method, params, sessionId) => cdpSend(ws, pending, nextId, method, params, sessionId);
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
  // A background tab gets no animation frames (`onFrame`, core 1.12, uses
  // requestAnimationFrame): make this tab the visible, focused one.
  await send("Page.bringToFront");
  await send("Emulation.setFocusEmulationEnabled", { enabled: true });
  /** One frame: a CDP session (out of process) or an execution context (in the page process). */
  const evalIn = async (t, expression) => {
    let res;
    try {
      res = await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true, contextId: t.contextId }, t.sessionId);
    } catch {
      return undefined; // the frame is not ready yet, or it was replaced
    }
    if (res.exceptionDetails) throw new Error(res.exceptionDetails.exception?.description ?? res.exceptionDetails.text);
    return res.result?.value;
  };
  /**
   * Evaluates `expression` in the newest app frame (undefined if there is
   * none or it fails). `match(appId)`: only a frame whose app (the
   * `data-app-id` of its body) passes.
   */
  const frameEval = async (expression, match) => {
    const targets = [...frames.map((sessionId) => ({ sessionId })), ...childContexts.map((contextId) => ({ contextId }))].reverse();
    for (const t of targets) {
      if (match && !match(await evalIn(t, `document.body?.dataset.appId ?? ""`).catch(() => undefined))) continue;
      return evalIn(t, expression);
    }
    return undefined;
  };
  return {
    frameEval,
    /** Polls `expression` in the app frame until it is not undefined/null, or throws. */
    async frameWaitFor(expression, timeoutMs, intervalMs = 200, match = undefined) {
      const deadline = Date.now() + timeoutMs;
      for (;;) {
        const v = await frameEval(expression, match).catch(() => undefined);
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
  const typingOnly = process.argv.includes("--typing-only");
  const exportOnly = process.argv.includes("--export-only");
  // `--only=<text>`: only the named checks whose name has <text> (no axe run, no export, no Hub).
  const only = process.argv.find((a) => a.startsWith("--only="))?.slice("--only=".length) ?? null;

  try {
    await waitForCdp(cdpPort);

    for (const app of hubOnly || typingOnly || exportOnly || only ? [] : APPS) {
      const appUrl = `${base}/examples/${app}/dist/${app.split("/").pop()}.plnt`;
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
    if (!exportOnly && !only) console.log("== typing in todo ==");
    if (!exportOnly && !only) try {
      await checkTyping(cdpPort, base);
      console.log("  focus and caret kept after each key: ok");
    } catch (err) {
      console.error(`  FAILED: ${err.stack ?? err}`);
      failed = true;
    }
    if (!exportOnly && !only) console.log("== one click stops the stopwatch ==");
    if (!exportOnly && !only) try {
      await checkStopwatch(cdpPort, base);
      console.log("  ok");
    } catch (err) {
      console.error(`  FAILED: ${err.stack ?? err}`);
      failed = true;
    }
    // The renderer updates elements in place (SPEC.md M5): hover, text
    // selection and unchanged elements survive re-renders.
    for (const [name, check] of exportOnly ? [] : [
      ["hover and text selection survive timer ticks (stopwatch)", checkHoverAndSelection],
      ["TextArea selection survives a re-render (settings-gallery)", checkTextAreaSelection],
      ["an unchanged row is the same element after a commit (todo)", checkRowIdentity],
      ["one row of 10,000 changes, no other element does (big-list)", checkBigList],
      ["Level 2 primitives: layout and keyboard (primitives)", checkPrimitives],
      ["absolute boxes and held keys (pong)", checkPong],
      ["canvas pointer positions (7GUIs circle drawer)", checkCircleDrawer],
      ["context menu at the pointer, Shift+F10, a modal blocks the pointer (circle drawer)", checkContextMenu],
      ["drag and drop, Alt+Up/Down, a form in a Dialog (dnd-fixture)", checkDragDrop],
      ["drag and drop in the sandboxed app frame (dnd-fixture as a web export)", checkDragDropFrame],
      ["host banners: an uncaught error, then a stopped app (stop-fixture)", checkStopBanner],
      ["system settings: dark, high contrast, forced colors, reduced motion, text size (todo)", checkSystemSettings],
    ].filter(([name]) => !only || name.includes(only))) {
      console.log(`== ${name} ==`);
      try {
        const note = await check(cdpPort, base);
        console.log(`  ok${note ? `: ${note}` : ""}`);
      } catch (err) {
        console.error(`  FAILED: ${err.stack ?? err}`);
        failed = true;
      }
    }
    // The web export (SPEC.md §10.3): `plinth build --target web`.
    if (!typingOnly && !hubOnly && !only) {
      console.log("== web export (plinth build --target web) ==");
      try {
        const r = await checkWebExport(cdpPort);
        const serious = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
        summary.push({ app: "web export: notes", serious: serious.length, minor: r.violations.length - serious.length });
        for (const v of r.violations) console.log(`  [${v.impact}] ${v.id}: ${v.help} (${v.nodes.length} node(s))`);
        if (serious.length > 0) failed = true;
        console.log(`  ok: ${r.note}`);
      } catch (err) {
        console.error(`  FAILED: ${err.stack ?? err}`);
        failed = true;
      }
    }
    // plinth:notify through the frame and the page (export of notify-demo).
    if (!typingOnly && !hubOnly && !only) {
      console.log("== notifications through the page (notify-demo export) ==");
      try {
        console.log(`  ok: ${await checkNotify(cdpPort)}`);
      } catch (err) {
        console.error(`  FAILED: ${err.stack ?? err}`);
        failed = true;
      }
    }
    // The web App Hub (docs/web-hub.md): served by `plinth registry serve --web`.
    if (!typingOnly && !exportOnly && !only) console.log("== web App Hub ==");
    if (!typingOnly && !exportOnly && !only) try {
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
    await closeEdge(cdpPort, edge);
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
function frameSurface(page, match = undefined, selector = ".host-frame") {
  return {
    eval: (e) => page.frameEval(e, match),
    waitFor: (e, t) => page.frameWaitFor(e, t, 200, match),
    offset: () => page.eval(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.x, y: r.y }; })()`),
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
  // Start: a new out-of-process app frame can miss the first synthetic
  // click in headless Edge, so click Start again until the stopwatch runs.
  // The check is about Stop, which must work with one click.
  for (let attempt = 1; ; attempt++) {
    await mouseClick(page, surface, button("Start"), 150);
    await new Promise((res) => setTimeout(res, 400));
    const before = await surface.eval(shown);
    await new Promise((res) => setTimeout(res, 200));
    if ((await surface.eval(shown)) !== before) break;
    if (attempt === 3) throw new Error(`the stopwatch did not start: ${JSON.stringify(await surface.eval(`({ text: document.getElementById("app").innerText.slice(0, 120), start: !!(${button("Start")}) })`))}`);
  }
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

/** Opens `examples/<app>` in the web host and waits for `ready` (an expression). */
async function openApp(cdpPort, base, app, ready) {
  const appUrl = `${base}/examples/${app}/dist/${app.split("/").pop()}.plnt`;
  const coreUrl = `${base}/target/core.wasm`;
  const page = await openPage(cdpPort);
  await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(coreUrl)}`);
  try {
    await page.waitFor(`(${ready}) ? true : null`, 30000);
  } catch (err) {
    await page.close();
    throw err;
  }
  return page;
}

const sleep = (ms) => new Promise((res) => setTimeout(res, ms));

/**
 * With the stopwatch running (10 commits a second), the mouse rests on
 * Reset and the "Actions" legend text is selected. After many ticks, Reset
 * must be the same element and still `:hover`, and the selection must
 * still hold the same text.
 */
async function checkHoverAndSelection(cdpPort, base) {
  const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === ${JSON.stringify(label)})`;
  const page = await openApp(cdpPort, base, "timer", button("Start"));
  try {
    await page.eval(`${button("Start")}.click()`);
    const r = await page.eval(`(() => { const b = ${button("Reset")}; window.__reset = b; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: r.x, y: r.y });
    await page.eval(`(() => {
      const legend = [...document.querySelectorAll("#app legend")].find((l) => l.textContent === "Actions");
      const range = document.createRange();
      range.selectNodeContents(legend);
      getSelection().removeAllRanges();
      getSelection().addRange(range);
    })()`);
    const shown = `[...document.querySelectorAll("#app .pl-heading")].map((h) => h.textContent).join()`;
    const before = await page.eval(shown);
    await sleep(600);
    if ((await page.eval(shown)) === before) throw new Error("the stopwatch did not tick");
    const state = await page.eval(`({
      same: ${button("Reset")} === window.__reset,
      hover: window.__reset.matches(":hover"),
      selection: getSelection().toString(),
    })`);
    if (!state.same) throw new Error("the Reset button is a new element after the ticks");
    if (!state.hover) throw new Error("Reset lost its :hover state");
    // (The legend's CSS shows it in capitals; toString() gives what is shown.)
    if (state.selection.toLowerCase() !== "actions") throw new Error(`the selection is "${state.selection}", not "Actions"`);
  } finally {
    await page.close();
  }
}

/**
 * Types into the settings gallery's TextArea, selects a part of the text,
 * then a click on a Checkbox makes the app commit (a Text changes). The
 * TextArea must keep the focus and the selection.
 */
async function checkTextAreaSelection(cdpPort, base) {
  const page = await openApp(cdpPort, base, "settings-gallery", `document.querySelector("#app textarea")`);
  try {
    await page.eval(`(window.__ta = document.querySelector("#app textarea")).focus()`);
    await page.send("Input.insertText", { text: "hello world" });
    await page.eval(`window.__ta.setSelectionRange(6, 11, "forward")`);
    const textBefore = await page.eval(`document.querySelector("#app main").textContent`);
    // A programmatic click does not move the focus.
    await page.eval(`document.querySelector("#app input[type=checkbox]").click()`);
    await sleep(100);
    if ((await page.eval(`document.querySelector("#app main").textContent`)) === textBefore) throw new Error("the checkbox click did not change the page");
    const state = await page.eval(`(() => { const t = document.querySelector("#app textarea"); return { same: t === window.__ta, focused: document.activeElement === t, value: t.value, start: t.selectionStart, end: t.selectionEnd }; })()`);
    if (!state.same) throw new Error("the TextArea is a new element");
    if (!state.focused) throw new Error("the TextArea lost the focus");
    if (state.value !== "hello world" || state.start !== 6 || state.end !== 11) throw new Error(`TextArea state after the commit: ${JSON.stringify(state)}`);
  } finally {
    await page.close();
  }
}

/** Adds a task in todo: the rows that were there are the same DOM objects. */
async function checkRowIdentity(cdpPort, base) {
  const page = await openApp(cdpPort, base, "todo", `document.querySelector("#app input[type=text]")`);
  try {
    const add = async (text) => {
      await page.eval(`document.querySelector("#app input[type=text]").focus()`);
      await page.send("Input.insertText", { text });
      await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
      await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    };
    const rows = `[...document.querySelectorAll("#app .pl-row")]`;
    await add("first");
    await page.waitFor(`${rows}.length >= 1 ? true : null`, 3000);
    const count = await page.eval(`(window.__rows = ${rows}).length`);
    await add("second");
    await page.waitFor(`${rows}.length > ${count} ? true : null`, 3000);
    const kept = await page.eval(`window.__rows.every((r) => r.isConnected && ${rows}.includes(r))`);
    if (!kept) throw new Error("adding a task made the old rows again");
    return `${count} old row(s) kept`;
  } finally {
    await page.close();
  }
}

/**
 * big-list (10,000 rows): clicks on one row's Toggle. Only that row may
 * change: a MutationObserver records every DOM change, and each one must be
 * inside the row (or the list's swap of that one row). The first and last
 * rows stay the same objects. Prints the time of each whole round trip
 * (the event, the guest's list diff, the commit and the DOM update).
 */
async function checkBigList(cdpPort, base) {
  const t0 = Date.now();
  const page = await openApp(cdpPort, base, "big-list", `document.querySelectorAll("#app .pl-row").length === 10000`);
  try {
    const loadMs = Date.now() - t0;
    const r = await page.eval(`(async () => {
      const rows = () => document.querySelectorAll("#app .pl-row");
      const first = rows()[0], last = rows()[9999], row = rows()[4], list = row.parentNode;
      const records = [];
      const mo = new MutationObserver((rs) => records.push(...rs));
      mo.observe(document.getElementById("app"), { subtree: true, childList: true, attributes: true, characterData: true });
      // Time the renderer's own part of each round trip too.
      const { DomRenderer } = await import("/web/dom-renderer.js");
      const renderMs = [];
      const render = DomRenderer.prototype.render;
      let onRender = null;
      DomRenderer.prototype.render = function () {
        const t = performance.now();
        render.call(this);
        renderMs.push(performance.now() - t);
        onRender?.();
      };
      const times = [];
      const seen = new Set([row]);
      for (let i = 0; i < 5; i++) {
        const target = rows()[4];
        seen.add(target);
        const t = performance.now();
        // The app runs in a Worker: the commit comes back after the click.
        const rendered = new Promise((res) => (onRender = res));
        target.querySelector("input[type=checkbox]").click();
        await Promise.race([rendered, new Promise((res) => setTimeout(res, 5000))]);
        onRender = null;
        times.push(performance.now() - t);
      }
      await new Promise((res) => setTimeout(res, 100));
      mo.disconnect();
      DomRenderer.prototype.render = render;
      seen.add(rows()[4]);
      const inRow = (n) => [...seen].some((r) => r.contains(n));
      const outside = records.filter((m) => !inRow(m.target) &&
        !(m.type === "childList" && m.target === list && [...m.addedNodes, ...m.removedNodes].every((n) => seen.has(n))));
      return {
        times,
        renderMs,
        records: records.length,
        outside: outside.slice(0, 5).map((m) => m.type + " on " + (m.target.className || m.target.nodeName)),
        outsideCount: outside.length,
        sameRow: rows()[4] === row,
        sameEnds: rows()[0] === first && rows()[9999] === last,
        on: rows()[4].querySelector("input[type=checkbox]").checked,
      };
    })()`);
    const ms = r.times.map((t) => t.toFixed(1)).join(", ");
    const renderMs = r.renderMs.map((t) => t.toFixed(2)).join(", ");
    console.log(`  load ${loadMs} ms; one-row toggle round trips: ${ms} ms (DOM renderer part: ${renderMs} ms); ${r.records} DOM mutation record(s), ${r.outsideCount} outside the row`);
    if (!r.sameEnds) throw new Error("the first or last row is a new element");
    if (r.outsideCount) throw new Error(`DOM changes outside the toggled row: ${r.outside.join("; ")}`);
    if (!r.on) throw new Error("the toggle is not on after 5 clicks");
    return `the row element itself ${r.sameRow ? "is kept" : "was made again by the guest"}`;
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

/** Opens todo and types into its TextField key by key (real input events). */
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

/**
 * The Level 2 primitives (UI API 1.6) in Edge: the mapped cards of a row
 * Scroll are in one row and the Scroll scrolls sideways; a Pressable is a
 * tab stop that Enter presses; a disabled one is not a tab stop.
 */
async function checkPrimitives(cdpPort, base) {
  const page = await openApp(cdpPort, base, "primitives", `document.querySelector("#app .pl-scroll .pl-pressable")`);
  try {
    const layout = await page.eval(`(() => {
      const cards = [...document.querySelectorAll("#app .pl-scroll .pl-pressable")];
      const tops = new Set(cards.map((c) => Math.round(c.getBoundingClientRect().top)));
      const s = document.querySelector("#app .pl-scroll");
      return { cards: cards.length, rows: tops.size, width: Math.round(cards[0].getBoundingClientRect().width), scrolls: s.scrollWidth > s.clientWidth };
    })()`);
    if (layout.cards !== 6 || layout.rows !== 1 || layout.width !== 160 || !layout.scrolls) throw new Error(`the row Scroll: ${JSON.stringify(layout)}`);
    const reset = `[...document.querySelectorAll("#app [role=link]")].find((e) => e.getAttribute("aria-label") === "Reset the count")`;
    if ((await page.eval(`${reset}.getAttribute("tabindex")`)) !== null) throw new Error("a disabled Pressable is a tab stop");
    await page.eval(`[...document.querySelectorAll("#app [role=button]")].find((e) => e.getAttribute("aria-label") === "Press me").focus()`);
    for (const type of ["keyDown", "keyUp"]) await page.send("Input.dispatchKeyEvent", { type, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await page.waitFor(`document.getElementById("app").innerText.includes("Pressed 1 times") ? true : null`, 3000);
    if ((await page.eval(`${reset}.getAttribute("tabindex")`)) !== "0") throw new Error("the Reset link is not a tab stop after a press");
    // UI API 1.10: the Canvas is an SVG image in the aspect of its view space.
    const svg = await page.eval(`(() => { const s = document.querySelector("#app svg.pl-canvas"); const r = s.getBoundingClientRect();
      return { role: s.getAttribute("role"), label: s.getAttribute("aria-label"), shapes: s.children.length, aspect: r.width / r.height }; })()`);
    if (svg.role !== "img" || svg.label !== "A sun over five bars" || svg.shapes !== 9 || Math.abs(svg.aspect - 2) > 0.02) {
      throw new Error(`the canvas: ${JSON.stringify(svg)}`);
    }
    // UI API 1.7: the hover style of a card (a generated class) colors its border.
    const inbox = `[...document.querySelectorAll("#app [role=button]")].find((e) => e.getAttribute("aria-label") === "Inbox")`;
    const borderOf = `getComputedStyle(${inbox}).borderTopColor`;
    const before = await page.eval(borderOf);
    const c = await page.eval(`(() => { ${inbox}.scrollIntoView({ block: "center" }); const r = ${inbox}.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: c.x, y: c.y });
    const hovered = await page.eval(borderOf);
    const accent = await page.eval(`(() => { const d = document.createElement("div"); d.style.color = "var(--pl-accent)"; document.body.append(d); const v = getComputedStyle(d).color; d.remove(); return v; })()`);
    if (hovered === before || hovered !== accent) {
      const info = await page.eval(`JSON.stringify({ cls: ${inbox}.className, rules: [...(document.getElementById("pl-partial-styles")?.sheet?.cssRules ?? [])].map((r) => r.cssText), hover: ${inbox}.matches(":hover") })`);
      throw new Error(`the hover style did not apply: ${before} -> ${hovered} (accent ${accent}) ${info}`);
    }
    await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: 1, y: 1 });
    // The compact width class: the header box becomes a column.
    const direction = `getComputedStyle([...document.querySelectorAll("#app .pl-box")].find((b) => b.textContent.includes("Styled primitives") && b.textContent.includes("U1"))).flexDirection`;
    if ((await page.eval(direction)) !== "row") throw new Error("the header box is not a row at the regular width");
    await page.send("Emulation.setDeviceMetricsOverride", { width: 400, height: 900, deviceScaleFactor: 1, mobile: false });
    await page.waitFor(`${direction} === "column" ? true : null`, 3000).catch(async () => {
      throw new Error(`the compact style did not apply: ${await page.eval(direction)} at ${await page.eval("innerWidth")} px`);
    });
    await page.send("Emulation.clearDeviceMetricsOverride");
    if (process.env.A11Y_SHOTS) {
      await mkdir(process.env.A11Y_SHOTS, { recursive: true });
      const r = await page.send("Page.captureScreenshot", { format: "png" });
      await writeFile(path.join(process.env.A11Y_SHOTS, "primitives.png"), Buffer.from(r.data, "base64"));
    }
    return `${layout.cards} cards in one row, Enter presses, disabled is not a tab stop, hover style, compact style`;
  } finally {
    await page.close();
  }
}

/**
 * Pong in Edge (UI API 1.9): the ball and the paddles are absolute boxes
 * in the court; real key events on the focused court start the game
 * (Space) and move the paddle while ArrowUp is held, and stop it when the
 * key is released.
 */
async function checkPong(cdpPort, base) {
  const page = await openApp(cdpPort, base, "pong", `document.querySelector('#app [aria-label^="Court"]')`);
  const key = async (type, key, code, vk) => page.send("Input.dispatchKeyEvent", { type, key, code, windowsVirtualKeyCode: vk });
  try {
    const paddle = `document.querySelector('#app [aria-label="Your paddle"]')`;
    const pos = await page.eval(`getComputedStyle(${paddle}).position + " " + getComputedStyle(document.querySelector('#app [aria-label="Ball"]')).position`);
    if (pos !== "absolute absolute") throw new Error(`the paddle and the ball are not absolute: ${pos}`);
    await page.eval(`document.querySelector('#app [aria-label^="Court"]').focus()`);
    await key("keyDown", " ", "Space", 32);
    await key("keyUp", " ", "Space", 32);
    await page.waitFor(`document.getElementById("app").innerText.includes("Rally") ? true : null`, 3000);
    const top = () => page.eval(`${paddle}.getBoundingClientRect().top`);
    const before = await top();
    await key("keyDown", "ArrowUp", "ArrowUp", 38);
    await sleep(300);
    await key("keyUp", "ArrowUp", "ArrowUp", 38);
    const held = await top();
    if (!(held < before)) throw new Error(`holding ArrowUp did not move the paddle up: ${before} -> ${held}`);
    await sleep(200);
    if ((await top()) !== held) throw new Error("the paddle moved after the key was released");
    // UI API 1.12: a mouse drag on the court aims the paddle (its center
    // goes to the pointer), also when the drag leaves the court.
    const court = await page.eval(`(() => { const r = document.querySelector('#app [aria-label^="Court"]').getBoundingClientRect(); return { x: r.left, y: r.top, h: r.height }; })()`);
    const mouse = (type, y, buttons) =>
      page.send("Input.dispatchMouseEvent", { type, x: court.x + 60, y, button: "left", buttons, clickCount: type === "mouseMoved" ? 0 : 1 });
    const near = (want) => `Math.abs(${paddle}.getBoundingClientRect().top - ${court.y + want}) < 1.5 ? true : null`;
    await mouse("mousePressed", court.y + 20, 1);
    await page.waitFor(near(0), 3000).catch(async () => {
      throw new Error(`a press near the top did not move the paddle to the top: ${(await top()) - court.y} px`);
    });
    await mouse("mouseMoved", court.y + 150, 1);
    await page.waitFor(near(130), 3000).catch(async () => {
      throw new Error(`the paddle did not follow the drag to 130 px: ${(await top()) - court.y} px`);
    });
    await mouse("mouseMoved", court.y + court.h + 24, 1);
    await page.waitFor(near(court.h - 40), 3000).catch(async () => {
      throw new Error(`a drag below the court did not take the paddle to the bottom: ${(await top()) - court.y} px`);
    });
    await mouse("mouseReleased", court.y + court.h + 24, 0);
    // Core 1.12 (`isActive`): a hidden page pauses the game (the `lifecycle`
    // event from `visibilitychange`). Headless Edge keeps a background tab
    // "visible", so the page says that it is hidden.
    await page.eval(`(() => { Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "hidden" });
      document.dispatchEvent(new Event("visibilitychange")); })()`);
    await page.waitFor(`document.getElementById("app").innerText.includes("Paused") ? true : null`, 5000).catch(() => {
      throw new Error("a hidden page did not pause the game");
    });
    return `moved ${Math.round(before - held)} px while ArrowUp was held; a mouse drag aims the paddle; a hidden page pauses`;
  } finally {
    await page.close();
  }
}

/**
 * UI API 1.12 on a Canvas: a real click on the SVG gives the position in
 * view units, so the new circle is drawn where the click was.
 */
async function checkCircleDrawer(cdpPort, base) {
  const svg = `document.querySelector('#app svg.pl-canvas')`;
  const page = await openApp(cdpPort, base, "7guis/circle-drawer", svg);
  try {
    const box = await page.eval(`(() => { const r = ${svg}.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width }; })()`);
    const click = async (dx, dy) => {
      for (const type of ["mouseMoved", "mousePressed", "mouseReleased"]) {
        await page.send("Input.dispatchMouseEvent", { type, x: box.x + dx, y: box.y + dy, button: "left", buttons: type === "mousePressed" ? 1 : 0, clickCount: 1 });
      }
    };
    await click(box.w / 4, 40);
    // The view is 400 units wide: a click at a quarter of the width is x 100.
    const k = 400 / box.w;
    const want = { cx: 100, cy: 40 * k };
    const got = await page.waitFor(`(() => { const c = ${svg}.querySelector("circle"); return c ? { cx: +c.getAttribute("cx"), cy: +c.getAttribute("cy") } : null; })()`, 3000);
    if (Math.abs(got.cx - want.cx) > 1 || Math.abs(got.cy - want.cy) > 1) throw new Error(`the circle is at ${JSON.stringify(got)}, not ${JSON.stringify(want)}`);
    return `a click at ${Math.round(box.w / 4)} px drew a circle at view x ${got.cx.toFixed(1)}`;
  } finally {
    await page.close();
  }
}

/**
 * UI API 1.13 (PRODUCTION U6): a right-click on a circle of the circle
 * drawer fires `onContextMenu`; the browser's menu is prevented only where
 * the app handles the event; the app's `Menu` opens at the pointer with the
 * focus on its first item, and Escape closes it. Shift+F10 on the focused
 * canvas opens it at the center. While the modal `Dialog` (with the slider) is open, a click
 * and a right-click on the canvas do nothing (GAPS G4).
 */
async function checkContextMenu(cdpPort, base) {
  const svg = `document.querySelector('#app svg.pl-canvas')`;
  const popover = `document.querySelector('#app .pl-menu-popover')`;
  const page = await openApp(cdpPort, base, "7guis/circle-drawer", svg);
  try {
    const box = await page.eval(`(() => { const r = ${svg}.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; })()`);
    const mouse = async (button, dx, dy) => {
      const x = box.x + dx;
      const y = box.y + dy;
      await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", x, y });
      await page.send("Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, buttons: button === "left" ? 1 : 2, clickCount: 1 });
      await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, buttons: 0, clickCount: 1 });
    };
    const outlines = `${svg}.querySelectorAll("circle[style*=stroke]").length`;
    // Was the browser's menu prevented? Recorded after the app's listener ran.
    await page.eval(`(() => { window.__ctx = []; window.addEventListener("contextmenu", (e) => setTimeout(() => window.__ctx.push(e.defaultPrevented)), true); })()`);
    const lastCtx = async () => {
      await sleep(100);
      return page.eval(`window.__ctx[window.__ctx.length - 1]`);
    };

    // A circle, then a right-click on it.
    await mouse("left", box.w / 2, box.h / 2);
    await page.waitFor(`${outlines} === 1 ? true : null`, 3000);
    await mouse("right", box.w / 2 + 3, box.h / 2 + 2);
    await page.waitFor(`${popover} && !${popover}.hidden ? true : null`, 3000).catch(() => {
      throw new Error("a right-click on a circle did not open the menu");
    });
    if ((await lastCtx()) !== true) throw new Error("the browser menu was not prevented on the canvas");
    const at = await page.eval(`(() => { const r = ${popover}.getBoundingClientRect(); return { x: r.left, y: r.top }; })()`);
    if (Math.abs(at.x - (box.x + box.w / 2 + 3)) > 1.5 || Math.abs(at.y - (box.y + box.h / 2 + 2)) > 1.5) {
      throw new Error(`the menu is at ${JSON.stringify(at)}, not at the pointer`);
    }
    await page.waitFor(`document.activeElement?.textContent === "Adjust diameter..." ? true : null`, 3000).catch(() => {
      throw new Error("the open menu does not have the focus on its first item");
    });
    if ((await page.eval(outlines)) !== 1) throw new Error("the right-click drew a circle");
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await page.waitFor(`${popover}.hidden ? true : null`, 3000);

    // A right-click where the app has no handler: the browser's menu stays.
    const heading = await page.eval(`(() => { const r = document.querySelector('#app h1, #app h2').getBoundingClientRect(); return { x: r.left + 4, y: r.top + r.height / 2 }; })()`);
    await page.send("Input.dispatchMouseEvent", { type: "mousePressed", x: heading.x, y: heading.y, button: "right", buttons: 2, clickCount: 1 });
    await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", x: heading.x, y: heading.y, button: "right", buttons: 0, clickCount: 1 });
    if ((await lastCtx()) !== false) throw new Error("the browser menu was prevented where the app has no handler");
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });

    // Shift+F10 on the focused canvas: the menu at its center (the selected circle).
    await page.eval(`${svg}.focus()`);
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "F10", code: "F10", windowsVirtualKeyCode: 121, modifiers: 8 });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "F10", code: "F10", windowsVirtualKeyCode: 121, modifiers: 8 });
    await page.waitFor(`${popover} && !${popover}.hidden ? true : null`, 3000).catch(() => {
      throw new Error("Shift+F10 on the canvas did not open the menu");
    });
    const center = await page.eval(`(() => { const r = ${popover}.getBoundingClientRect(); return { x: r.left, y: r.top }; })()`);
    // The focus can scroll the page: measure the canvas again.
    const now = await page.eval(`(() => { const r = ${svg}.getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    if (Math.abs(center.x - now.x) > 1.5 || Math.abs(center.y - now.y) > 1.5) {
      throw new Error(`Shift+F10 put the menu at ${JSON.stringify(center)}, not at the center of the canvas ${JSON.stringify(now)}`);
    }
    await page.waitFor(`document.activeElement?.textContent === "Adjust diameter..." ? true : null`, 3000);
    // Enter on the item opens the modal dialog.
    await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: String.fromCharCode(13) });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    await page.waitFor(`document.querySelector('#app dialog[open]') ? true : null`, 3000).catch(() => {
      throw new Error("the menu item did not open the dialog");
    });
    if (!(await page.eval(`${popover}.hidden`))) throw new Error("the menu did not close after its action");

    // The modal dialog blocks the pointer to the canvas under it.
    Object.assign(box, await page.eval(`(() => { const r = ${svg}.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; })()`));
    await mouse("left", 20, 20);
    await mouse("right", box.w / 2, box.h / 2);
    await sleep(300);
    if ((await page.eval(outlines)) !== 1) throw new Error("a click under the modal dialog drew a circle");
    if (!(await page.eval(`${popover}.hidden`))) throw new Error("a right-click under the modal dialog opened the menu");
    return "a right-click and Shift+F10 open the menu at the point; the browser menu stays where there is no handler; the modal dialog blocks the pointer";
  } finally {
    await page.close();
  }
}

/**
 * UI API 1.13 (PRODUCTION U6 part 2) with real pointer events: a drag of a
 * row onto another row of a `List` with `onReorder` moves it, and the drop
 * line shows; a drag of an element with `dragValue` onto an element with
 * `onDrop` gives the value, and the target shows the drop; Alt+Up on the
 * focused row moves it and keeps the focus; a finger drags after a long
 * press; a `Dialog` holds a form (the first field has the focus, Enter in a
 * field does not close it) and blocks a drag under it. `surface` is the
 * page or the sandboxed app frame. The same facts as
 * `crates/plinth-shoot/tests/drag_drop.rs`.
 */
async function dragDropChecks(page, surface) {
  const text = (prefix) => `[...document.querySelectorAll("#app .pl-text")].map((e) => e.textContent).find((t) => t.startsWith(${JSON.stringify(prefix)})) ?? ""`;
  const row = (title) => `[...document.querySelectorAll("#app .pl-row")].find((r) => r.querySelector(".pl-row-title")?.textContent === ${JSON.stringify(title)})`;
  const labeled = (label) => `document.querySelector('#app [aria-label=${JSON.stringify(label)}]')`;
  const live = `document.querySelector('#app [role=status]')?.textContent ?? ""`;
  const wait = (expr, want, ms = 3000) =>
    surface.waitFor(`(${expr}) === ${JSON.stringify(want)} ? true : null`, ms).catch(async () => {
      throw new Error(`waited for ${JSON.stringify(want)}, got ${JSON.stringify(await surface.eval(expr))}`);
    });
  await surface.waitFor(`${row("Apple")} ? true : null`, 20000);
  const point = async (find, dx = 0.5, dy = 0.5) => {
    const o = await surface.offset();
    const r = await surface.eval(`(() => { const r = ${find}.getBoundingClientRect(); return { x: r.x + r.width * ${dx}, y: r.y + r.height * ${dy} }; })()`);
    return { x: r.x + o.x, y: r.y + o.y };
  };
  const mouse = (type, p, buttons) => page.send("Input.dispatchMouseEvent", { type, x: p.x, y: p.y, button: "left", buttons, clickCount: 1 });
  /** Presses at `a`, moves to `b` in steps; `during` runs before the release. */
  const drag = async (a, b, during = async () => {}) => {
    await mouse("mouseMoved", a, 0);
    await mouse("mousePressed", a, 1);
    for (let i = 1; i <= 5; i++) await mouse("mouseMoved", { x: a.x + ((b.x - a.x) * i) / 5, y: a.y + ((b.y - a.y) * i) / 5 }, 1);
    await sleep(50);
    await during();
    await mouse("mouseReleased", b, 0);
  };

  // A row onto the last row: the line shows under it; the app gets [0, 2].
  await drag(await point(row("Apple")), await point(row("Cherry")), async () => {
    if (!(await surface.eval(`${row("Cherry")}.classList.contains("pl-drop-after")`))) throw new Error("no drop line under the target row");
    if ((await surface.eval(live)) !== "Dragging Apple") throw new Error(`the live region says ${JSON.stringify(await surface.eval(live))}`);
  });
  await wait(text("order: "), "order: Banana,Cherry,Apple");
  await wait(text("log: "), "log: reorder 0 2");
  await wait(live, "Moved Apple to position 3 of 3");
  if (await surface.eval(`!!document.querySelector("#app .pl-drop-after, #app .pl-drag-ghost")`)) throw new Error("the drop line or the drag card stays after the drop");

  // An element with dragValue onto a drop target: [value, x, y]; the target shows the drop.
  await drag(await point(labeled("Card"), 0.2, 0.3), await point(labeled("Basket")), async () => {
    if (!(await surface.eval(`${labeled("Basket")}.classList.contains("pl-drop-over")`))) throw new Error("the drop target does not show the drag over it");
  });
  await wait(text("log: "), "log: drop card true true");
  await wait(live, "Dropped Card");

  // The keyboard: Alt+Up on the focused row moves it; the focus stays on it.
  await surface.eval(`${row("Cherry")}.focus()`);
  const altKey = async (key, code) => {
    await page.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, code, windowsVirtualKeyCode: code === "ArrowUp" ? 38 : 40, modifiers: 1 });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: code === "ArrowUp" ? 38 : 40, modifiers: 1 });
  };
  await altKey("ArrowUp", "ArrowUp");
  await wait(text("order: "), "order: Cherry,Banana,Apple");
  await wait(`document.activeElement?.querySelector(".pl-row-title")?.textContent ?? ""`, "Cherry");
  await wait(live, "Moved Cherry to position 1 of 3");
  if ((await surface.eval(`${row("Cherry")}.getAttribute("aria-keyshortcuts")`)) !== "Alt+ArrowUp Alt+ArrowDown") throw new Error("the row does not tell its keys");
  await altKey("ArrowDown", "ArrowDown");
  await wait(text("order: "), "order: Banana,Cherry,Apple");

  // A finger: a long press, then a move, drags the row (Apple, the last row, onto Banana, the first).
  const touch = async (type, p) => page.send("Input.dispatchTouchEvent", { type, touchPoints: p ? [{ x: p.x, y: p.y, id: 1 }] : [] });
  const from = await point(row("Apple"));
  const to = await point(row("Banana"));
  await touch("touchStart", from);
  await sleep(600);
  for (let i = 1; i <= 5; i++) await touch("touchMove", { x: from.x + ((to.x - from.x) * i) / 5, y: from.y + ((to.y - from.y) * i) / 5 });
  await touch("touchEnd");
  await wait(text("order: "), "order: Apple,Banana,Cherry");
  await wait(text("log: "), "log: reorder 2 0");
  // A finger that moves at once scrolls: no drag.
  const quick = await point(row("Cherry"));
  await touch("touchStart", quick);
  for (let i = 1; i <= 5; i++) await touch("touchMove", { x: quick.x, y: quick.y - (from.y - to.y) * (i / 5) - 10 * i });
  await touch("touchEnd");
  await sleep(300);
  if ((await surface.eval(text("order: "))) !== "order: Apple,Banana,Cherry") throw new Error("a finger that moved at once dragged a row");

  // A Dialog with a form: the first field has the focus; Enter in the text field does not close it.
  await surface.eval(`[...document.querySelectorAll("#app button")].find((b) => b.textContent === "Open dialog").click()`);
  await surface.waitFor(`document.querySelector("#app dialog[open]") ? true : null`, 3000);
  await wait(`document.activeElement?.type ?? ""`, "range");
  const order = `[...document.querySelector("#app dialog[open]").querySelectorAll(".pl-dialog-message, input, .pl-action")].map((e) => e.className.includes("message") ? "message" : e.type === "range" ? "slider" : e.type === "text" ? "field" : "action").join()`;
  if ((await surface.eval(order)) !== "message,slider,field,action") throw new Error(`the dialog shows ${await surface.eval(order)}`);
  await surface.eval(`document.querySelector("#app dialog[open] input[type=text]").focus()`);
  await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
  await sleep(200);
  if (!(await surface.eval(`!!document.querySelector("#app dialog[open]")`))) throw new Error("Enter in a field of the dialog closed it");
  // The modal dialog blocks a drag under it.
  const before = await surface.eval(text("log: "));
  await drag(await point(labeled("Card"), 0.2, 0.3), await point(labeled("Basket")));
  await sleep(300);
  if ((await surface.eval(text("log: "))) !== before) throw new Error(`a drag under the dialog did something: ${await surface.eval(text("log: "))}`);
  await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
  await wait(text("log: "), "log: closed");
  return "a drag reorders and drops (mouse and finger), Alt+Up/Down, the drop line and target, a form in a Dialog, a modal blocks a drag";
}

async function checkDragDrop(cdpPort, base) {
  const appUrl = `${base}/web/test/fixtures/dnd-fixture/dist/dnd-fixture.plnt`;
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(`${base}/target/core.wasm`)}`);
    return await dragDropChecks(page, pageSurface(page));
  } finally {
    await page.close();
  }
}

/** The same checks in the sandboxed app frame of a web export (`<iframe sandbox="allow-scripts">`). */
async function checkDragDropFrame(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-dnd-"));
  const folder = path.join(dir, "dnd-web");
  execFileSync(exe, ["build", path.join(root, "web/test/fixtures/dnd-fixture"), "--target", "web", "--out", folder], { stdio: "ignore" });
  const server = await startServer(folder);
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`http://127.0.0.1:${server.address().port}/`);
    await page.waitFor(`document.querySelector("plinth-app")?.dataset.state === "running" ? true : null`, 20000);
    return await dragDropChecks(page, frameSurface(page, undefined, "plinth-app > iframe"));
  } finally {
    await page.close();
    server.close();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}

/**
 * The host-owned banners (dom-renderer.js `renderBanner`): an uncaught
 * error shows "An error occurred" with Dismiss, and the app keeps running;
 * a trap shows "This app stopped" with the core's reason, and the screen
 * takes no more input (`inert`).
 */
async function checkStopBanner(cdpPort, base) {
  const appUrl = `${base}/web/test/fixtures/stop-fixture/dist/stop-fixture.plnt`;
  const page = await openPage(cdpPort);
  try {
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(`${base}/target/core.wasm`)}`);
    const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent === ${JSON.stringify(label)})`;
    await page.waitFor(`${button("throw")} ? true : null`, 30000);
    const banner = `(() => { const b = document.querySelector(".pl-host-banner"); return b && !b.hidden ? b.textContent : null; })()`;
    await page.eval(`${button("throw")}.click()`);
    const error = await page.waitFor(banner, 3000);
    if (!error.includes("An error occurred") || !error.includes("Uncaught Error: on purpose")) throw new Error(`error banner: ${error}`);
    await page.eval(`document.querySelector(".pl-host-banner-dismiss").click()`);
    await page.waitFor(`document.querySelector(".pl-host-banner").hidden ? true : null`, 3000);
    await page.eval(`${button("ping")}.click()`);
    await page.waitFor(`document.querySelector("#app").textContent.includes("pong") ? true : null`, 3000);
    await page.eval(`${button("index")}.click()`);
    const stopped = await page.waitFor(banner, 3000);
    if (!stopped.includes("This app stopped") || !stopped.includes("array index out of bounds")) throw new Error(`stopped banner: ${stopped}`);
    const inert = await page.eval(`document.querySelector(".pl-screen > :not(.pl-host-banner)").inert`);
    if (!inert) throw new Error("the screen of a stopped app still takes input");

    // The time limit (GAPS R1): an endless loop runs in the app's Worker;
    // the page stays responsive, and after 2 s the watchdog ends the Worker
    // and shows the desktop's reason.
    await page.navigate(`${base}/web/index.html?app=${encodeURIComponent(appUrl)}&core=${encodeURIComponent(`${base}/target/core.wasm`)}`);
    await page.waitFor(`${button("loop")} ? true : null`, 30000);
    const t0 = Date.now();
    await page.eval(`${button("loop")}.click()`);
    await sleep(500);
    const t1 = Date.now();
    const busy = await page.eval(`(() => { let n = 0; for (let i = 0; i < 1000; i++) n += i; return document.querySelector("#app button") ? n : null; })()`);
    const evalMs = Date.now() - t1;
    if (busy !== 499500 || evalMs > 1000) throw new Error(`the page did not answer while the app loops (${evalMs} ms)`);
    const limited = await page.waitFor(banner, 10000);
    const ms = Date.now() - t0;
    if (!limited.includes("This app stopped") || !limited.includes("the app is not responding (time limit reached)")) throw new Error(`time limit banner: ${limited}`);
    if (ms < 1900) throw new Error(`the time limit stopped the app after ${ms} ms, before 2 s`);
    return `${stopped}; an endless loop: the page answered in ${evalMs} ms, stopped after ${ms} ms (${limited})`;
  } finally {
    await page.close();
  }
}

/**
 * The web export (SPEC.md §10.3). The folder form of notes on a plain static
 * server (no CORS headers): `<plinth-app>` runs the app in a sandboxed
 * frame, a note goes into the `store.sql` database of the page origin
 * (SQLite in a Worker of the page, OPFS) and is there again after a reload. The single-file form of todo from disk (file://):
 * the app runs and typing works in the frame. Returns the axe result of
 * the notes frame.
 */
async function checkWebExport(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-export-"));
  const folder = path.join(dir, "notes-web");
  execFileSync(exe, ["build", path.join(root, "examples/notes"), "--target", "web", "--out", folder], { stdio: "ignore" });
  const single = path.join(dir, "todo.html");
  execFileSync(exe, ["build", path.join(root, "examples/todo"), "--target", "web", "--single-file", "--out", single], { stdio: "ignore" });
  const server = await startServer(folder);
  const base = `http://127.0.0.1:${server.address().port}`;
  const state = `(() => { const e = document.querySelector("plinth-app"); const s = e?.dataset.state;
    if (s === "failed") throw new Error("the app did not start: " + e.textContent); return s === "running" ? true : null; })()`;
  const page = await openPage(cdpPort);
  const frame = frameSurface(page);
  try {
    await page.navigate(`${base}/`);
    await page.waitFor(state, 20000);
    await frame.waitFor(`[...document.querySelectorAll("#app button")].some((b) => b.textContent.trim() === "Add note") ? true : null`, 10000).catch(async (err) => {
      throw new Error(`${err.message}; the frame shows: ${JSON.stringify(await page.frameEval(`document.body?.innerHTML.slice(0, 300)`))}`);
    });
    const violations = await axeRun(page, (e) => page.frameEval(e));
    await frame.eval(`(() => {
      const set = (label, v) => { const i = [...document.querySelectorAll("#app input, #app textarea")].find((e) => e.labels?.[0]?.textContent.trim() === label || e.getAttribute("aria-label") === label);
        i.value = v; i.dispatchEvent(new Event("input", { bubbles: true })); };
      set("Title", "Exported note"); set("Body", "From the web export test");
    })()`);
    // The app runs in a Worker: the commit that enables "Add note" comes after the input events.
    const addNote = `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === "Add note")`;
    await frame.waitFor(`${addNote} && !${addNote}.disabled ? true : null`, 5000);
    await frame.eval(`${addNote}.click()`);
    await frame.waitFor(`document.getElementById("app").innerText.includes("Exported note") ? true : null`, 5000);
    // store.sql: the export runs SQLite in a Worker of the page, with OPFS of the page origin.
    const opfs = await page.eval(`(async () => { try { const d = await navigator.storage.getDirectory(); await d.getDirectoryHandle(".plinth-sql"); return true; } catch { return false; } })()`);
    if (!opfs) throw new Error("the SQL database of the export is not in OPFS (no .plinth-sql folder)");
    const height = await page.eval(`document.querySelector("plinth-app > iframe").getBoundingClientRect().height`);
    if (height < 200) throw new Error(`the app frame is ${height} px high; height="fill" must give it the page`);
    await page.navigate(`${base}/`);
    await page.waitFor(state, 20000);
    await frame.waitFor(`document.getElementById("app").innerText.includes("Exported note") ? true : null`, 10000);

    await page.navigate(pathToFileURL(single).href);
    await page.waitFor(state, 20000);
    await typeKeys(page, frame, () => frame.eval(`document.querySelector("#app input[type=text]").focus()`));
    return { violations, note: "notes from a static server (store.sql in OPFS, kept across a reload), todo from one file on disk" };
  } finally {
    await page.close();
    server.close();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}

/**
 * `plinth:notify` (core 1.12) in Edge: notify-demo as a web export, so the
 * app runs in the sandboxed frame and the page shows the notification
 * (`frame-host.js`, `browserNotify`). "Remind me" (a real click) shows a
 * notification after 5 s. First with no permission: the result must be a
 * defined answer (headless Edge does not ask the user), and the app must
 * not stop. Then with the permission given through DevTools: "shown".
 */
async function checkNotify(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-notify-"));
  const folder = path.join(dir, "notify-web");
  execFileSync(exe, ["build", path.join(root, "examples/notify-demo"), "--target", "web", "--out", folder], { stdio: "ignore" });
  const server = await startServer(folder);
  const base = `http://127.0.0.1:${server.address().port}`;
  const page = await openPage(cdpPort);
  const frame = frameSurface(page);
  const remind = `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === "Remind me")`;
  const result = `(() => { const t = document.getElementById("app").innerText;
    if (t.includes("This app stopped")) throw new Error("the app stopped: " + t);
    const m = t.match(/The notification is shown\\.|This device cannot show notifications\\.|No notification: [^\\n]*/); return m ? m[0] : null; })()`;
  const once = async () => {
    await page.navigate(`${base}/`);
    await frame.waitFor(`${remind} ? true : null`, 20000);
    const permission = await page.eval(`Notification.permission`);
    await mouseClick(page, frame, remind);
    // A click in the frame also activates the page (User Activation v2).
    const activated = await page.eval(`navigator.userActivation?.isActive ?? null`);
    await frame.waitFor(`document.getElementById("app").innerText.includes("A reminder comes") ? true : null`, 5000).catch(async () => {
      // A new out-of-process frame can miss the first synthetic click.
      await mouseClick(page, frame, remind);
    });
    const answer = await frame.waitFor(result, 15000);
    return { answer, permission, activated };
  };
  try {
    const first = await once();
    await page.send("Browser.grantPermissions", { origin: base, permissions: ["notifications"] });
    const granted = await once();
    if (granted.answer !== "The notification is shown.") throw new Error(`with the permission: ${granted.answer}`);
    return `no permission (${first.permission}, page activated by the frame click: ${first.activated}): "${first.answer}"; with the permission: "${granted.answer}"`;
  } finally {
    await page.close();
    server.close();
    await rm(dir, { recursive: true, force: true }).catch(() => {});
  }
}

/**
 * The theme follows the system settings (SPEC.md §6.5, PRODUCTION U7): dark
 * mode, high contrast (`prefers-contrast: more`, with text at least 7:1),
 * forced colors (the browser colors win, controls keep a border), reduced
 * motion (no transitions) and a larger default font size (all type is in
 * rem). DevTools `Emulation.setEmulatedMedia` gives each setting, so the
 * result does not depend on the settings of this machine. axe has no
 * serious violation in dark and in high contrast.
 */
async function checkSystemSettings(cdpPort, base) {
  const page = await openApp(cdpPort, base, "todo", `document.querySelector("#app .pl-section .pl-button")`);
  const media = (features) => page.send("Emulation.setEmulatedMedia", { features });
  const DEFAULTS = [
    { name: "prefers-color-scheme", value: "light" },
    { name: "prefers-contrast", value: "no-preference" },
    { name: "forced-colors", value: "none" },
    { name: "prefers-reduced-motion", value: "no-preference" },
  ];
  const withMedia = (over) => DEFAULTS.map((d) => over.find((o) => o.name === d.name) ?? d);
  // The colors of the page, and the WCAG contrast of the text and the muted
  // text on the background.
  const look = () => page.eval(`(() => {
    const rgb = (c) => (c.match(/[0-9.]+/g) ?? []).slice(0, 3).map(Number);
    const lum = (c) => { const [r, g, b] = rgb(c).map((v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
    const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };
    const body = getComputedStyle(document.body);
    const section = getComputedStyle(document.querySelector("#app .pl-section"));
    const muted = getComputedStyle(document.querySelector("#app .pl-section-title, #app .pl-field-label"));
    const button = document.querySelector("#app .pl-button, #app .pl-action");
    const b = getComputedStyle(button);
    const r = button.getBoundingClientRect();
    return { bg: body.backgroundColor, text: body.color, border: section.borderTopColor, borderWidth: section.borderTopWidth,
      textRatio: ratio(body.color, body.backgroundColor), mutedRatio: ratio(muted.color, body.backgroundColor),
      button: { border: b.borderTopStyle, borderWidth: parseFloat(b.borderTopWidth), w: r.width, h: r.height } };
  })()`);
  try {
    const out = [];
    // Dark and light.
    await media(withMedia([{ name: "prefers-color-scheme", value: "light" }]));
    const light = await look();
    if (light.bg !== "rgb(245, 246, 248)") throw new Error(`light: the background is ${light.bg}`);
    await media(withMedia([{ name: "prefers-color-scheme", value: "dark" }]));
    const dark = await look();
    if (dark.bg !== "rgb(11, 12, 15)") throw new Error(`dark: the background is ${dark.bg}`);
    const axeSerious = async (name) => {
      const v = (await axeRun(page)).filter((x) => x.impact === "serious" || x.impact === "critical");
      if (v.length) throw new Error(`${name}: axe ${v.map((x) => `${x.id} (${x.nodes.map((n) => n.target).join(" ")})`).join(", ")}`);
    };
    await axeSerious("dark");
    out.push("dark and light follow prefers-color-scheme");
    // High contrast, dark and light: 7:1, borders in the text color, 2 px.
    for (const scheme of ["dark", "light"]) {
      await media(withMedia([{ name: "prefers-color-scheme", value: scheme }, { name: "prefers-contrast", value: "more" }]));
      const hc = await look();
      const want = scheme === "dark" ? ["rgb(0, 0, 0)", "rgb(255, 255, 255)"] : ["rgb(255, 255, 255)", "rgb(0, 0, 0)"];
      if (hc.bg !== want[0] || hc.border !== want[1] || hc.borderWidth !== "2px") throw new Error(`high contrast ${scheme}: ${JSON.stringify(hc)}`);
      if (hc.textRatio < 7 || hc.mutedRatio < 7) throw new Error(`high contrast ${scheme}: text ${hc.textRatio.toFixed(2)}:1, muted ${hc.mutedRatio.toFixed(2)}:1`);
      const ring = await page.eval(`(() => { const b = document.querySelector("#app .pl-input"); b.focus({ focusVisible: true }); const w = getComputedStyle(b).outlineWidth; b.blur(); return w; })()`);
      if (ring !== "3px") throw new Error(`high contrast ${scheme}: the focus ring is ${ring}`);
      await axeSerious(`high contrast ${scheme}`);
    }
    out.push("high contrast 7:1");
    // Forced colors: the system colors win; buttons keep a visible border.
    await media(withMedia([{ name: "forced-colors", value: "active" }]));
    const forced = await look();
    const canvas = await page.eval(`(() => { const d = document.createElement("div"); d.style.background = "Canvas"; d.style.color = "CanvasText"; document.body.append(d); const s = getComputedStyle(d); const v = [s.backgroundColor, s.color]; d.remove(); return v; })()`);
    if (forced.bg !== canvas[0] || forced.text !== canvas[1]) throw new Error(`forced colors: the page is ${forced.bg}/${forced.text}, the system colors ${canvas}`);
    if (forced.button.border !== "solid" || forced.button.borderWidth < 1 || forced.button.w < 1 || forced.button.h < 1) throw new Error(`forced colors: the button is not visible: ${JSON.stringify(forced.button)}`);
    out.push("forced colors");
    // Reduced motion: no transitions.
    const probe = `(() => { let d = document.getElementById("pl-motion-probe"); if (!d) { d = document.createElement("div"); d.id = "pl-motion-probe"; d.style.transition = "opacity 1s"; document.body.append(d); } return getComputedStyle(d).transitionDuration; })()`;
    await media(withMedia([]));
    if ((await page.eval(probe)) !== "1s") throw new Error(`no preference: the transition is ${await page.eval(probe)}`);
    await media(withMedia([{ name: "prefers-reduced-motion", value: "reduce" }]));
    if ((await page.eval(probe)) !== "0s") throw new Error(`reduced motion: the transition is ${await page.eval(probe)}`);
    out.push("reduced motion");
    await media(withMedia([]));
    // Text size: a default font size of 32 px (200 %) doubles the type.
    const sizes = `(() => ({ title: parseFloat(getComputedStyle(document.querySelector("#app .pl-title")).fontSize),
      button: parseFloat(getComputedStyle(document.querySelector("#app .pl-button, #app .pl-action")).fontSize),
      overflow: document.documentElement.scrollWidth > innerWidth + 1 }))()`;
    const normal = await page.eval(sizes);
    await page.send("Page.setFontSizes", { fontSizes: { standard: 32 } });
    await page.eval(`location.reload()`);
    await page.waitFor(`document.querySelector("#app .pl-section") ? true : null`, 30000);
    const large = await page.eval(sizes);
    if (Math.abs(large.title / normal.title - 2) > 0.05 || Math.abs(large.button / normal.button - 2) > 0.05) {
      throw new Error(`text size 200 %: ${JSON.stringify({ normal, large })}`);
    }
    if (large.overflow) throw new Error("text size 200 %: the page scrolls sideways");
    out.push(`text ${normal.title}px -> ${large.title}px`);
    return out.join(", ");
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

const HUB_APPS = ["counter", "notes", "budget", "utility", "todo", "timer", "calculator", "files-demo", "hub"];
const ID = (app) => (app === "hub" ? "dev.plinth.hub" : app.endsWith("-fixture") ? `dev.plinth.test.${app}` : `dev.plinth.examples.${app}`);

/**
 * The web App Hub in headless Edge (docs/web-hub.md): `hub.html` runs the
 * Hub app (`examples/hub`, signed with a throwaway key that the registry's
 * hub.json trusts) from the registry. Its library is the registry listing.
 * Open in the Hub app launches each app in the host's app window, a
 * sandboxed iframe (opaque origin): counter at once; utility after the
 * host's consent window; notes and budget each with their own kv data,
 * which the other cannot see; a grant that the Hub app turns off is denied
 * in the app; a package that does not match the registry digest does not
 * open; typing in todo and the stopwatch work in the frame. Light, dark and
 * phone width. Returns the axe result of each view.
 */
async function checkHub(cdpPort) {
  const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");
  const dir = await mkdtemp(path.join(tmpdir(), "plinth-a11y-hub-"));
  const reg = path.join(dir, "registry");
  await mkdir(reg);
  for (const app of HUB_APPS) {
    await copyFile(path.join(root, "examples", app, "dist", `${app}.plnt`), path.join(reg, `${app}.plnt`));
  }
  // A second app with `plinth:files`, for the isolation check.
  await copyFile(path.join(root, "web/test/fixtures/files-fixture/dist/files-fixture.plnt"), path.join(reg, "files-fixture.plnt"));
  // An app with `store.sql`: SQLite with OPFS in a Worker of the page.
  await copyFile(path.join(root, "web/test/fixtures/sql-fixture/dist/sql-fixture.plnt"), path.join(reg, "sql-fixture.plnt"));
  const env = { ...process.env, PLINTH_PUBLISHER_DIR: path.join(dir, "publisher") };
  execFileSync(exe, ["publisher", "init", "--name", "you"], { env, stdio: "ignore" });
  const key = execFileSync(exe, ["publisher", "show"], { env, encoding: "utf8" }).trim().split(/\s+/).pop();
  execFileSync(exe, ["sign", path.join(reg, "hub.plnt")], { env, stdio: "ignore" });
  execFileSync(exe, ["registry", "build", reg, "--with-core", "--hub-trusted-key", key], { stdio: "ignore" });
  // Change one byte of the calculator package after the build: the registry
  // digest no longer agrees, so the host must refuse to run it.
  const calcDoc = JSON.parse(await readFile(path.join(reg, "apps", ID("calculator"), "index.json"), "utf8"));
  const calcPkg = path.join(reg, "packages", `${calcDoc.versions[0].sha256}.plnt`);
  const bytes = await readFile(calcPkg);
  bytes[bytes.length - 30] ^= 1;
  await writeFile(calcPkg, bytes);

  const { child, base } = await startHubServer(exe, reg);
  const results = [];
  const page = await openPage(cdpPort);
  // The Hub app runs in its own sandboxed frame (#hub-frame); a launched
  // app runs in the frame of the host's app window.
  const HUB = ID("hub");
  const hub = frameSurface(page, (id) => id === HUB, "#hub-frame");
  const frame = frameSurface(page, (id) => Boolean(id) && id !== HUB, "#host-app .host-frame");
  try {
    // A11Y_SHOTS=<folder>: also save a PNG of each view, to look at by eye.
    const shot = async (name) => {
      if (!process.env.A11Y_SHOTS) return;
      await mkdir(process.env.A11Y_SHOTS, { recursive: true });
      const r = await page.send("Page.captureScreenshot", { format: "png" });
      await writeFile(path.join(process.env.A11Y_SHOTS, `hub-${name}.png`), Buffer.from(r.data, "base64"));
    };
    // The host page (without frames: dialogs, the app window) and the Hub app frame.
    const hubAxe = async (view) =>
      results.push({ view, violations: [...(await axeRun(page, undefined, { iframes: false })), ...(await axeRun(page, hub.eval))] });
    const appText = () => hub.eval(`document.getElementById("app").innerText`);
    const button = (label) => `[...document.querySelectorAll("#app button")].find((b) => b.textContent.trim() === ${JSON.stringify(label)})`;
    /** Clicks in the Hub app. */
    const click = async (expr, what) => {
      await hub.waitFor(`${expr} ? true : null`, 10000).catch(() => {
        throw new Error(`no ${what}`);
      });
      await hub.eval(`${expr}.click()`);
    };
    const running = (id) =>
      `document.body.dataset.running === ${JSON.stringify(id)} ? true : (document.body.dataset.notice || document.body.dataset.failed ? (() => { throw new Error("the app did not open: " + (document.body.dataset.notice || document.body.dataset.failed)); })() : null)`;
    const waitRunning = async (id) => {
      await page.waitFor(running(id), 20000).catch(async (err) => {
        const state = await page.eval(`JSON.stringify({ data: { ...document.body.dataset }, windows: document.querySelectorAll("#host-app").length, frames: document.querySelectorAll("iframe").length, last: document.querySelector("iframe")?.dataset.last, consent: !!document.getElementById("host-consent") })`);
        const inFrame = await frame.eval(`JSON.stringify({ text: document.body.innerText.slice(0, 200), data: { ...document.body.dataset } })`).catch((e) => String(e));
        throw new Error(`${id} did not start: ${state} frame: ${inFrame} (${err.message.slice(0, 60)})`);
      });
    };
    const library = async () => {
      if (await hub.eval(`!!document.querySelector("#app .pl-back")`)) await hub.eval(`document.querySelector("#app .pl-back").click()`);
      await click(`[...document.querySelectorAll("#app .pl-nav-item")].find((b) => b.textContent === "Your apps")`, "Your apps tab");
      await hub.waitFor(`document.querySelector("#app .pl-row") ? true : null`, 10000);
    };
    /** Selects the app in the Hub app's library and presses Open. */
    const openApp = async (name) => {
      await library();
      await click(`[...document.querySelectorAll("#app .pl-row")].find((r) => r.querySelector(".pl-row-title")?.textContent === ${JSON.stringify(name)})`, `row ${name}`);
      await hub.waitFor(`document.querySelector("#app h1")?.textContent.includes(${JSON.stringify(name)}) ? true : null`, 10000);
      await click(button("Open"), "Open button");
    };
    const closeApp = async () => {
      await page.eval(`document.getElementById("host-app-close")?.click()`);
      await page.waitFor(`document.getElementById("host-app") ? null : true`, 5000);
    };

    for (const scheme of ["light", "dark"]) {
      await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
      await page.navigate(`${base}/`);
      await page.waitFor(`document.body.dataset.ready === "true" || document.body.dataset.startError ? true : null`, 20000);
      const startError = await page.eval(`document.body.dataset.startError ?? ""`);
      if (startError) throw new Error(`the Hub did not start: ${startError}`);
      if ((await page.eval(`document.getElementById("hub-frame")?.getAttribute("sandbox")`)) !== "allow-scripts") throw new Error("the Hub app is not in a sandboxed frame");
      await hub.waitFor(`document.querySelectorAll("#app .pl-row").length > 0 ? true : null`, 10000);
      const rows = await hub.eval(`[...document.querySelectorAll("#app .pl-row-title")].map((t) => t.textContent).join(",")`);
      // By app id: the examples, then the test fixture.
      if (rows !== "Budget,Calculator,Counter,Files,Notes,Stopwatch,Todo,Utility,Files Fixture,SQL Fixture") {
        throw new Error(`the Hub library shows ${rows}`);
      }
      await hubAxe(`hub library (${scheme})`);
      await shot(`library-${scheme}`);
    }
    await page.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });

    // Discover: search the registry.
    await click(`[...document.querySelectorAll("#app .pl-nav-item")].find((b) => b.textContent === "Store")`, "Store tab");
    await hub.waitFor(`document.querySelector("#app input[type=text]") ? true : null`, 10000);
    await hub.eval(`(() => { const i = document.querySelector("#app input[type=text]"); i.value = "util"; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
    await click(button("Search"), "Search button");
    await hub.waitFor(`document.getElementById("app").innerText.includes("1 app found.") ? true : null`, 10000);
    if (!(await appText()).includes("Utility")) throw new Error(`search "util" does not find Utility:\n${await appText()}`);
    await hubAxe("hub discover");

    // Counter (no capabilities): it opens at once, in the sandboxed frame.
    await openApp("Counter");
    await waitRunning(ID("counter"));
    const sandbox = await page.eval(`document.querySelector("#host-app .host-frame").getAttribute("sandbox")`);
    if (sandbox !== "allow-scripts") throw new Error(`the app frame has sandbox="${sandbox}"`);
    await frame.waitFor(`document.querySelector("#app button") ? true : null`, 20000);
    // The frame has an opaque origin: no storage, no cookies, no hub DOM.
    const walls = await frame.eval(`(() => {
      const tryIt = (f) => { try { f(); return "open"; } catch { return "blocked"; } };
      return { origin: self.origin, storage: tryIt(() => localStorage.length), cookie: tryIt(() => document.cookie),
               parent: tryIt(() => parent.document.body), idb: tryIt(() => indexedDB.open("x")) };
    })()`);
    if (walls.origin !== "null" || walls.storage !== "blocked" || walls.cookie !== "blocked" || walls.parent !== "blocked") {
      throw new Error(`the app frame is not isolated: ${JSON.stringify(walls)}`);
    }
    await frame.eval(`${button("Increment")}.click()`);
    await frame.waitFor(`[...document.querySelectorAll("#app h1, #app h2")].some((h) => h.textContent.trim() === "1") ? true : null`, 5000);
    await hubAxe("app window (counter)");
    results.push({ view: "counter in the app frame", violations: await axeRun(page, frame.eval) });
    await shot("counter");
    await closeApp();

    // Utility: clipboard.read (medium) is not decided: the host's consent window first.
    await openApp("Utility");
    await page.waitFor(`document.body.dataset.consent === ${JSON.stringify(ID("utility"))} ? true : null`, 10000);
    const consent = await page.eval(`document.getElementById("host-consent").innerText`);
    for (const want of ["Allow Utility to do these things?", "Read the clipboard.", "Medium risk", "Paste text into the input box."]) {
      if (!consent.includes(want)) throw new Error(`the consent window has no "${want}":\n${consent}`);
    }
    if (consent.includes("Write to the clipboard")) throw new Error("the consent window asks for a low-risk capability");
    const checked = await page.eval(`document.querySelector("#host-consent input[data-cap='clipboard.read']:checked")?.value`);
    if (checked !== "deny") throw new Error(`clipboard.read (medium risk) is not "deny" first: ${checked}`);
    await hubAxe("consent window (utility)");
    await shot("consent-utility");
    await page.eval(`document.getElementById("host-consent-open").click()`);
    await page.waitFor(`document.getElementById("host-consent") ? null : true`, 5000).catch(async () => {
      const st = await page.eval(`(() => { const d = document.getElementById("host-consent"); const b = document.getElementById("host-consent-open"); return JSON.stringify({ open: d.open, rv: d.returnValue, n: document.querySelectorAll("dialog").length, btn: b.outerHTML, form: !!b.form, active: document.activeElement?.outerHTML?.slice(0, 80) }); })()`);
      throw new Error(`the consent window did not close: ${st}`);
    });
    await waitRunning(ID("utility"));
    await closeApp();
    // No question the second time.
    await openApp("Utility");
    await waitRunning(ID("utility"));
    if (await page.eval(`!!document.getElementById("host-consent")`)) throw new Error("the consent window came again");
    await closeApp();
    // After Refresh, the Hub app shows the decision in the label.
    await library();
    await click(`[...document.querySelectorAll("#app .pl-action")].find((b) => b.textContent === "Refresh")`, "Refresh action");
    await click(`[...document.querySelectorAll("#app .pl-row")].find((r) => r.querySelector(".pl-row-title")?.textContent === "Utility")`, "row Utility");
    await hub.waitFor(`document.getElementById("app").innerText.includes("Not allowed") ? true : null`, 5000).catch(async () => {
      throw new Error(`the Hub app does not show the refused grant:\n${await appText()}`);
    });

    // Notes: its notes are in its store.sql database (OPFS of the hub page),
    // and it writes nothing to kv.
    await openApp("Notes");
    await waitRunning(ID("notes"));
    await frame.waitFor(`document.body.dataset.kvEntries === "0" ? true : null`, 5000);
    await frame.eval(`(() => {
      const set = (label, v) => { const i = [...document.querySelectorAll("#app input, #app textarea")].find((e) => e.labels?.[0]?.textContent.trim() === label || e.getAttribute("aria-label") === label);
        i.value = v; i.dispatchEvent(new Event("input", { bubbles: true })); };
      set("Title", "Isolated note"); set("Body", "From the a11y test");
    })()`);
    // The app runs in a Worker: the commit that enables "Add note" comes after the input events.
    await frame.waitFor(`${button("Add note")} && !${button("Add note")}.disabled ? true : null`, 5000);
    await frame.eval(`${button("Add note")}.click()`);
    await frame.waitFor(`document.getElementById("app").innerText.includes("Isolated note") ? true : null`, 5000);
    const keysOf = (id) => `Object.keys(localStorage).filter((k) => k.startsWith("plinth-hub:kv:${id}:"))`;
    await closeApp();

    // Budget: its frame gets none of the notes data.
    const budgetBefore = await page.eval(`${keysOf(ID("budget"))}.length`);
    await openApp("Budget");
    await waitRunning(ID("budget"));
    const budgetGot = await frame.waitFor(`document.body.dataset.kvEntries ?? null`, 5000);
    if (Number(budgetGot) !== budgetBefore) throw new Error(`budget got ${budgetGot} kv entries, not ${budgetBefore} (the notes data must not be there)`);
    if ((await frame.eval(`document.getElementById("app").innerText`)).includes("Isolated note")) throw new Error("budget shows the note of notes");
    await closeApp();

    // Notes again: the note is there (the page keeps the database).
    await openApp("Notes");
    await frame.waitFor(`document.getElementById("app").innerText.includes("Isolated note") ? true : null`, 10000);
    await closeApp();

    // The Hub app turns store.sql off for notes: notes runs and reads nothing.
    await hub.eval(`[...document.querySelectorAll("#app label.pl-toggle")].find((l) => l.textContent.includes("Allow: Keep a database on this device")).querySelector("input").click()`);
    await hub.waitFor(`document.getElementById("app").innerText.includes("Not allowed") ? true : null`, 5000);
    await click(button("Open"), "Open button");
    await waitRunning(ID("notes"));
    await frame.waitFor(`document.querySelector("#app button") ? true : null`, 10000);
    if ((await frame.eval(`document.getElementById("app").innerText`)).includes("Isolated note")) throw new Error("notes reads its data with store.sql denied");
    await closeApp();

    // plinth:files: Files saves a file; the page keeps it in its IndexedDB,
    // in the namespace of Files; Files Fixture (another app) cannot list or
    // read it.
    await openApp("Files");
    await waitRunning(ID("files-demo"));
    await frame.waitFor(`document.querySelector("#app textarea") ? true : null`, 10000);
    await frame.eval(`(() => {
      const set = (label, v) => { const i = [...document.querySelectorAll("#app input, #app textarea")].find((e) => e.labels?.[0]?.textContent.trim() === label || e.getAttribute("aria-label") === label);
        i.value = v; i.dispatchEvent(new Event("input", { bubbles: true })); };
      set("File name", "secret.md"); set("Text", "A file of Files");
    })()`);
    await frame.eval(`${button("Save")}.click()`);
    await frame.waitFor(`document.getElementById("app").innerText.includes("Saved secret.md.") ? true : null`, 10000).catch(async () => {
      throw new Error(`Files did not save: ${await frame.eval(`document.getElementById("app").innerText`)}`);
    });
    const idbKeys = () =>
      page.eval(`new Promise((resolve, reject) => { const r = indexedDB.open("plinth-files", 1); r.onupgradeneeded = () => r.result.createObjectStore("files");
        r.onsuccess = () => { const q = r.result.transaction("files").objectStore("files").getAllKeys(); q.onsuccess = () => { resolve(JSON.stringify(q.result)); r.result.close(); }; q.onerror = () => reject(q.error); };
        r.onerror = () => reject(r.error); })`);
    const keys = await idbKeys();
    if (keys !== JSON.stringify([[ID("files-demo"), "notes/secret.md"]])) throw new Error(`the page's files are ${keys}`);
    await closeApp();
    await openApp("Files Fixture");
    await waitRunning(ID("files-fixture"));
    const fixture = async (label, want) => {
      await frame.waitFor(`${button(label)} ? true : null`, 10000);
      // The app runs in a Worker: wait for a new result, not the one of the last button.
      const done = `(document.getElementById("app").innerText.match(/done:[^\\n]*/) ?? [""])[0]`;
      const before = await frame.eval(done);
      await frame.eval(`${button(label)}.click()`);
      await frame.waitFor(`${done} && ${done} !== ${JSON.stringify(before)} ? true : null`, 10000).catch(() => {});
      const got = (await frame.eval(`document.getElementById("app").innerText`)).match(/done:[^\n]*/)[0];
      if (got !== want) throw new Error(`Files Fixture ${label}: ${got}, not ${want}`);
    };
    await fixture("list", "done:list:0");
    await fixture("read", "done:err:not-found");
    await closeApp();
    // Files again: its file is still there (from the page).
    await openApp("Files");
    await waitRunning(ID("files-demo"));
    await frame.waitFor(`[...document.querySelectorAll("#app .pl-row-title")].some((t) => t.textContent === "secret.md") ? true : null`, 10000);
    await closeApp();

    // store.sql: the page runs SQLite in its own Worker with OPFS (the
    // frame has an opaque origin and no storage). The data stays after a
    // reload of the page; the authorizer and the time limit work.
    const press = async (label) => {
      await frame.waitFor(`${button(label)} ? true : null`, 10000);
      const done = `(document.getElementById("app").innerText.match(/done:[^\\n]*/) ?? [""])[0]`;
      const before = await frame.eval(done);
      await frame.eval(`${button(label)}.click()`);
      await frame.waitFor(`${done} && ${done} !== ${JSON.stringify(before)} ? true : null`, 15000).catch(() => {});
      return (await frame.eval(`document.getElementById("app").innerText`)).match(/done:[^\n]*/)?.[0] ?? "";
    };
    await openApp("SQL Fixture");
    await waitRunning(ID("sql-fixture"));
    let got = await press("add");
    if (got !== "done:rows:1") throw new Error(`SQL Fixture add: ${got}`);
    got = await press("refused");
    if (got !== "done:sql: not authorized;sql: not authorized;sql: the query took too long") throw new Error(`SQL Fixture refused: ${got}`);
    await closeApp();
    const opfs = await page.eval(`navigator.storage.getDirectory().then((d) => d.getDirectoryHandle(".plinth-sql")).then(() => "yes", (e) => String(e))`);
    if (opfs !== "yes") throw new Error(`no OPFS folder .plinth-sql: ${opfs}`);
    await page.navigate(`${base}/`);
    await page.waitFor(`document.body.dataset.ready === "true" ? true : null`, 20000);
    await openApp("SQL Fixture");
    await waitRunning(ID("sql-fixture"));
    got = await press("count");
    if (got !== "done:v2:rows:1:hits:1") throw new Error(`SQL Fixture after a reload: ${got}`);
    await closeApp();

    // A package that does not match the registry digest does not open.
    await openApp("Calculator");
    await page.waitFor(`document.body.dataset.notice ? true : null`, 10000);
    const notice = await page.eval(`document.body.dataset.notice`);
    if (!notice.includes("does not match the registry digest")) throw new Error(`no digest message: ${notice}`);
    if (await page.eval(`document.querySelector("#host-app .host-frame") !== null`)) throw new Error("a refused package has an app frame");
    await hubAxe("notice (digest)");
    await page.eval(`document.querySelector(".host-dialog button").click()`);
    await page.eval(`delete document.body.dataset.notice`);

    // Typing and the stopwatch inside the app frame.
    await openApp("Todo");
    await waitRunning(ID("todo"));
    await typeKeys(page, frame, async () => {
      // A real click first. Headless Edge does not always move the focus
      // into a new out-of-process frame on a synthetic click (the iframe
      // element gets the focus, the frame document does not); then focus
      // the field from inside the frame, as the standalone check does. The
      // keys then go to the field.
      const input = `document.querySelector("#app input[type=text]")`;
      await mouseClick(page, frame, input);
      const focused = `document.activeElement?.tagName === "INPUT" && document.hasFocus() ? true : null`;
      if (!(await frame.waitFor(focused, 1500).catch(() => false))) {
        await frame.eval(`(window.focus(), ${input}.focus())`);
        await frame.waitFor(focused, 2000);
      }
    });
    await closeApp();
    await openApp("Stopwatch");
    await waitRunning(ID("timer"));
    await stopwatchClicks(page, frame);
    await closeApp();

    // Phone width: no horizontal scroll. Last, because the device metrics
    // override does not reach the out-of-process app frames in the same way
    // (their clicks then miss).
    await page.send("Emulation.setDeviceMetricsOverride", { width: 390, height: 800, deviceScaleFactor: 2, mobile: true });
    await library();
    const overflow = Math.max(
      await page.eval(`document.documentElement.scrollWidth - window.innerWidth`),
      await hub.eval(`document.documentElement.scrollWidth - window.innerWidth`),
    );
    if (overflow > 0) throw new Error(`the Hub scrolls sideways by ${overflow}px at 390px`);
    await hubAxe("hub library (phone)");
    await shot("library-phone");
    await page.send("Emulation.clearDeviceMetricsOverride");
    console.log("  Hub app from the registry, Discover, sandboxed frame, consent window, kv isolation, files isolation, store.sql with OPFS after a reload, grant from the Hub app, digest check, typing and stopwatch in the frame, phone width: ok");
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
