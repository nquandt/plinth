// Node test: `plinth:clipboard` and `plinth:dialog` on the web host
// (`docs/host-apis.md` "clipboard", "dialog"), with a fixture app on the
// real core (`web/test/fixtures/clipboard-fixture`). A fake `clipboard`
// object stands in for `navigator.clipboard`; a fake `askDialog` stands in
// for `alert`/`confirm`/`prompt`. Checks: a write-then-read round trip
// (ASCII and Unicode), a clipboard call denied by refusal (no trap,
// `lastError()` reports it), and `alert`, `confirm` (true and false) and
// `prompt` (text and cancel) give the right results to the app.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm
//   target/debug/plinth.exe build web/test/fixtures/clipboard-fixture

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

/** A fake in-memory clipboard, shaped like `navigator.clipboard`'s
 * `writeText`/`readText` (both async), for the `clipboard` load option. */
function fakeClipboard() {
  let value = null;
  return {
    writeText: (t) => {
      value = t;
      return Promise.resolve();
    },
    readText: () => Promise.resolve(value),
  };
}

/** Runs one fixture app on the real core; returns it with a `press` helper
 * that clicks the button `name` and waits for "done:…" (as `run-files.mjs`
 * does for `plinth:files`). */
async function startApp(coreBytes, plnt, opts = {}) {
  const { appWasm, manifestText } = await readPlnt(plnt);
  const app = new PlinthApp();
  const text = new Map();
  const label = new Map();
  const listen = new Map();
  let resultId;
  let resultOps = 0;
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
        resultId ??= op.id;
        if (op.id === resultId) resultOps++;
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && op.prop === 3) {
        label.set(op.id, op.value);
      }
    }
  };
  await app.load(coreBytes, appWasm, { log: () => {}, reportError: (s) => app.errors.push(s), manifestText: opts.manifestText ?? manifestText, ...opts });
  app.errors = [];
  app.init([]);
  app.press = async (name) => {
    let handler;
    for (const [id, byEvent] of listen) if (label.get(id) === name) handler = byEvent.get(1);
    assert.ok(handler !== undefined, `no button ${name}`);
    const before = resultOps;
    const previous = text.get(resultId);
    app.onEvent({ kind: "ui", handler, event: 1, value: null });
    const start = Date.now();
    for (;;) {
      const now = text.get(resultId);
      if (now.startsWith("done:") && resultOps > before && now !== "done:") break;
      if (now === previous && now.startsWith("done:") && Date.now() - start > 300) break;
      if (Date.now() - start > 5000) throw new Error(`no result for ${name}: ${now}`);
      await new Promise((r) => setTimeout(r, 2));
    }
    assert.deepEqual(app.errors, [], "uncaught errors");
    return text.get(resultId).slice("done:".length);
  };
  return app;
}

async function clipboard() {
  const plnt = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/clipboard-fixture/dist/clipboard-fixture.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));

  // Write then read, including Unicode (accents, CJK, an emoji outside the BMP).
  const solo = await startApp(coreBytes, plnt, { clipboard: fakeClipboard() });
  assert.equal(await solo.press("write-then-read"), "héllo 世界 \u{1F44B}");

  // Denied by refusal: declared in the manifest, refused at run time. No
  // trap; `readText` answers `null`, `writeText` does nothing, and
  // `lastError()` reports the reason (`docs/host-apis.md` "clipboard").
  const refused = await startApp(coreBytes, plnt, { clipboard: fakeClipboard(), refused: new Set(["clipboard.write", "clipboard.read"]) });
  assert.equal(await refused.press("write-then-read-with-error"), "null/denied:refused");
}

async function dialogs() {
  const plnt = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/clipboard-fixture/dist/clipboard-fixture.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));

  const asked = [];
  const askDialog = (kind, message) => {
    asked.push([kind, message]);
    if (kind === "alert") return null;
    if (kind === "confirm") return message === "ok?" && askDialog.nextConfirm;
    return askDialog.nextPrompt;
  };

  const app = await startApp(coreBytes, plnt, { clipboard: fakeClipboard(), askDialog });

  assert.equal(await app.press("alert"), "alerted");

  askDialog.nextConfirm = true;
  assert.equal(await app.press("confirm-true"), "true");

  askDialog.nextConfirm = false;
  assert.equal(await app.press("confirm-false"), "false");

  askDialog.nextPrompt = "Ada";
  assert.equal(await app.press("prompt-text"), "Ada");

  askDialog.nextPrompt = null;
  assert.equal(await app.press("prompt-cancel"), "null");

  assert.deepEqual(
    asked.map((a) => a[0]),
    ["alert", "confirm", "confirm", "prompt", "prompt"],
  );
}

async function main() {
  await clipboard();
  await dialogs();
  console.log("run-clipboard-dialog.mjs: ok (clipboard write/read round trip incl. Unicode, denied by refusal, alert, confirm true/false, prompt text/cancel)");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
