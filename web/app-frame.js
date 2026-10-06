// The app frame of the web App Hub (docs/web-hub.md §4): `app-frame.html`
// runs in `<iframe sandbox="allow-scripts">`, so it has an opaque origin.
// It cannot read the storage, the cookies or the DOM of the hub page or of
// a different app. The hub page sends the package, the core, the kv snapshot
// of this app and the refused capabilities in one `start` message; the frame
// sends kv writes, clipboard and dialog requests back (`hub-storage.js`).

import { PlinthApp, readPlnt, parseManifest } from "./plinth-web.js";
import { Tree, DomRenderer } from "./dom-renderer.js";
import { CHANNEL, frameKvStore } from "./hub-storage.js";

const container = document.getElementById("app");
const embedded = window.parent !== window;
let parentOrigin = null;
let started = false;
let nextId = 1;
let hub = null; // the `plinth:hub` copy of the Hub app (`FrameHub`), else null
const pending = new Map(); // request id -> resolve

function send(msg) {
  // Before `start`, the frame does not know the origin of the hub page; the
  // first message (`ready`) carries no data.
  window.parent.postMessage({ channel: CHANNEL, ...msg }, parentOrigin ?? "*");
}

function ask(type, fields) {
  const id = nextId++;
  return new Promise((resolve) => {
    pending.set(id, resolve);
    send({ type, id, ...fields });
  });
}

function showError(text) {
  const pre = document.createElement("pre");
  pre.className = "pl-frame-error";
  pre.textContent = text;
  container.replaceChildren(pre);
}

async function start(msg) {
  const { appWasm, manifestText, assets } = await readPlnt(new Uint8Array(msg.pkg));
  const kvStore = frameKvStore(msg.kv, msg.quota, (m) => send(m));
  // For tests: which app this frame runs.
  document.body.dataset.appId = parseManifest(manifestText).id;
  if (msg.hub) {
    // Only the Hub app gets a snapshot. A web export (plinth.js) never
    // does, so the bundle never loads this module.
    const { FrameHub } = await import("./hub-host.js");
    hub = new FrameHub(msg.hub, { send, ask });
  }
  const app = new PlinthApp();
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(new Uint8Array(msg.core), appWasm, {
    log: (s) => console.log("[app]", s),
    manifestText,
    kvStore,
    refused: new Set(Array.isArray(msg.refused) ? msg.refused : []),
    askDialog: (kind, message) => ask("dialog", { kind, message: String(message) }),
    clipboard: {
      writeText: (text) => send({ type: "clipboard-write", text }),
      readText: () => ask("clipboard-read", {}),
    },
    // The parent page makes each request (it checks the capability again),
    // so a request carries the page's origin, not "null".
    netFetch: (url, method, headers, body) => ask("net-fetch", { url, method, headers, body }),
    hub,
  });
  new DomRenderer(tree, container, app, assets);
  app.init([]);
  // For tests (web/test/run-a11y.mjs): how many kv entries the start message had.
  document.body.dataset.kvEntries = String(Object.keys(msg.kv ?? {}).length);
  // The content height, so that a parent page can size the iframe to it.
  let last = -1;
  const report = () => {
    const height = Math.ceil(container.getBoundingClientRect().height);
    if (height !== last) {
      last = height;
      send({ type: "size", height });
    }
  };
  new ResizeObserver(report).observe(container);
  report();
}

window.addEventListener("message", (event) => {
  if (event.source !== window.parent) return;
  const msg = event.data;
  if (!msg || msg.channel !== CHANNEL) return;
  if (msg.type === "start" && !started) {
    started = true;
    // A page from disk (file://) has the origin "null", which is not a
    // valid target origin; "*" still reaches only the parent window.
    parentOrigin = event.origin === "null" ? "*" : event.origin;
    start(msg)
      .then(() => send({ type: "started" }))
      .catch((err) => {
        console.error(err);
        showError(String(err?.stack ?? err));
        send({ type: "failed", message: String(err?.message ?? err) });
      });
  } else if (msg.type === "answer" && (parentOrigin === "*" || event.origin === parentOrigin) && pending.has(msg.id)) {
    const resolve = pending.get(msg.id);
    pending.delete(msg.id);
    resolve(msg.value ?? null);
  } else if (msg.type === "hub-state" && hub && (parentOrigin === "*" || event.origin === parentOrigin)) {
    try {
      hub.applySnapshot({ state: msg.state });
    } catch (err) {
      console.error(err);
    }
  }
});

if (embedded) send({ type: "ready" });
else showError("This page runs one app for the web App Hub. Open the hub (hub.html) to run an app.");
