// Node test of `plinth:notify` on the web host (core 1.12, docs/host-apis.md
// "plinth:notify", PRODUCTION H3): `browserNotify` on a fake
// `Notification` (allowed, the browser denied, the permission question,
// unsupported, a tag that replaces, cancel), the bridge message checks,
// and examples/notify-demo on `PlinthApp` (shown, refused, unsupported,
// the browser denied, cancel). A denied call never traps.
// run-a11y.mjs runs notify-demo in Edge, through the frame and the page.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/notify-demo
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt, browserNotify } from "../plinth-web.js";
import { checkFrameMessage, CHANNEL, MAX_NOTIFY_TEXT } from "../hub-storage.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

/** A fake `Notification` class: `permission`, `requestPermission`, the made notifications. */
function fakeNotification(permission = "granted", ask = null) {
  class N {
    constructor(title, options) {
      if (N.throws) throw new TypeError("Illegal constructor");
      this.title = title;
      this.options = options;
      this.closed = false;
      N.made.push(this);
    }
    close() {
      this.closed = true;
      this.onclose?.();
    }
  }
  N.permission = permission;
  N.made = [];
  N.asked = 0;
  N.throws = false;
  N.requestPermission = async () => {
    N.asked++;
    if (ask instanceof Error) throw ask;
    N.permission = ask ?? N.permission;
    return N.permission;
  };
  return N;
}

const quiet = { origin: "http://example.com", log: () => {} };

// Allowed: shown, with body, tag and icon; a tag replaces; cancel closes.
{
  const N = fakeNotification("granted");
  const n = browserNotify(N, { ...quiet, icon: "icon.png" });
  assert.equal(await n.show("Hello", "Body", "t1"), "shown");
  assert.deepEqual(N.made[0].options, { body: "Body", tag: "t1", icon: "icon.png" });
  assert.equal(await n.show("Again", null, "t1"), "shown");
  assert.deepEqual(N.made[1].options, { tag: "t1", icon: "icon.png" }, "the same tag: the browser replaces the first");
  assert.equal(await n.show("No tag", null, null), "shown");
  n.cancel("t1");
  assert.equal(N.made[1].closed, true, "cancel closes the newest notification with the tag");
  assert.equal(N.made[0].closed, false);
  n.cancel("t1"); // already closed: not an error
  n.cancel("unknown");
  assert.equal(N.asked, 0, "no question when the permission is granted");
}

// The browser denied notifications for this page.
{
  const N = fakeNotification("denied");
  assert.equal(await browserNotify(N, quiet).show("x", null, null), "denied:refused");
  assert.equal(N.made.length, 0);
}

// The question: granted, denied, closed, or the browser does not ask.
for (const [answer, want] of [
  ["granted", "shown"],
  ["denied", "denied:refused"],
  ["default", "unsupported"],
  [new Error("no user gesture"), "unsupported"],
]) {
  const N = fakeNotification("default", answer);
  assert.equal(await browserNotify(N, quiet).show("x", null, null), want, `question answered ${answer}`);
  assert.equal(N.asked, 1);
}

// Unsupported: no API, an opaque origin, or the browser refuses the constructor.
{
  assert.equal(await browserNotify(undefined, quiet).show("x", null, null), "unsupported");
  const N = fakeNotification("granted");
  assert.equal(await browserNotify(N, { ...quiet, origin: "null" }).show("x", null, null), "unsupported");
  assert.equal(N.made.length, 0, "a sandboxed frame does not use the API");
  N.throws = true;
  assert.equal(await browserNotify(N, quiet).show("x", null, null), "unsupported");
}

// The bridge messages (frame to page).
{
  const msg = (fields) => ({ channel: CHANNEL, ...fields });
  assert.ok(checkFrameMessage(msg({ type: "notify-show", id: 1, title: "t", body: null, tag: null })));
  assert.ok(checkFrameMessage(msg({ type: "notify-show", id: 1, title: "t", body: "b", tag: "g" })));
  assert.equal(checkFrameMessage(msg({ type: "notify-show", id: 1, title: 5, body: null, tag: null })), null);
  assert.equal(checkFrameMessage(msg({ type: "notify-show", id: 1, title: "t", body: 3, tag: null })), null);
  assert.equal(checkFrameMessage(msg({ type: "notify-show", id: 1, title: "x".repeat(MAX_NOTIFY_TEXT + 1), body: null, tag: null })), null);
  assert.ok(checkFrameMessage(msg({ type: "notify-cancel", tag: "g" })));
  assert.equal(checkFrameMessage(msg({ type: "notify-cancel", tag: null })), null);
}

// examples/notify-demo on the web host's code. Its reminder waits 5 s:
// the test makes each timer of the app short.
const { appWasm, manifestText } = await readPlnt(new Uint8Array(readFileSync(path.join(root, "examples/notify-demo/dist/notify-demo.plnt"))));
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const realSetTimeout = globalThis.setTimeout;
globalThis.setTimeout = (fn, ms, ...args) => realSetTimeout(fn, Math.min(ms ?? 0, 10), ...args);

async function run(opts) {
  const app = new PlinthApp();
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  app.onStop = (reason) => assert.fail(`notify-demo stopped: ${reason}`);
  await app.load(core, appWasm, { log: () => {}, manifestText, ...opts });
  app.init([]);
  const nodes = () => [...tree.nodes.values()];
  const status = () => nodes().find((n) => n.kind === ControlKind.text)?.text;
  const press = (label) => {
    const b = nodes().find((n) => n.kind === ControlKind.button && n.props.get(Prop.label) === label);
    assert.ok(b, `a button ${label}`);
    app.onEvent({ kind: "ui", handler: b.listeners.get(Event.press), event: Event.press, value: null });
  };
  press("Remind me");
  const end = Date.now() + 5000;
  while (!/shown|cannot|No notification/.test(status() ?? "") && Date.now() < end) await new Promise((r) => realSetTimeout(r, 5));
  return { status: status(), press, app };
}

{
  const N = fakeNotification("granted");
  const r = await run({ notify: browserNotify(N, quiet) });
  assert.equal(r.status, "The notification is shown.");
  assert.equal(N.made.length, 1);
  assert.equal(N.made[0].title, "Reminder");
  assert.deepEqual(N.made[0].options, { body: "Your 5 seconds are up.", tag: "reminder" });
  r.press("Cancel");
  await new Promise((res) => realSetTimeout(res, 20));
  assert.equal(N.made[0].closed, true, "Cancel removes the notification");
}
{
  const N = fakeNotification("denied");
  const r = await run({ notify: browserNotify(N, quiet) });
  assert.equal(r.status, "No notification: denied:refused");
}
{
  const N = fakeNotification("granted");
  const r = await run({ notify: browserNotify(N, quiet), refused: new Set(["notify"]) });
  assert.equal(r.status, "No notification: denied:refused");
  assert.equal(N.made.length, 0, "a refused capability never reaches the browser");
  r.press("Cancel"); // a denied cancel does nothing
}
{
  const r = await run({});
  assert.equal(r.status, "This device cannot show notifications.");
}

globalThis.setTimeout = realSetTimeout;
console.log("run-notify.mjs: ok (shown, tag, cancel, browser denied, question, unsupported, bridge checks, notify-demo)");
