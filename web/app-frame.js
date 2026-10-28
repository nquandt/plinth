// The app frame of the web App Hub (docs/web-hub.md §4): `app-frame.html`
// runs in `<iframe sandbox="allow-scripts">`, so it has an opaque origin.
// It cannot read the storage, the cookies or the DOM of the hub page or of
// a different app. The hub page sends the package, the core, the kv snapshot
// of this app and the refused capabilities in one `start` message; the frame
// sends kv writes, clipboard and dialog requests back (`hub-storage.js`).
// The core and the app run in a dedicated Worker (`app-worker.js`); this
// document keeps only the DOM renderer and relays the host calls of the
// Worker that need the page.

import { readPlnt, parseManifest, normalizeLocale, pageLocale } from "./plinth-web.js";
import { Tree, DomRenderer } from "./dom-renderer.js";
import { CHANNEL, frameKvStore, frameDataVersionStore } from "./hub-storage.js";
import { WorkerApp, startAppWorker } from "./app-worker.js";
import { overLimit } from "./secrets.js";

const container = document.getElementById("app");
const embedded = window.parent !== window;
let parentOrigin = null;
let started = false;
let nextId = 1;
let app = null; // the `WorkerApp`: the app runs in its own Worker (app-worker.js)
let renderer = null; // the `DomRenderer`, after `start`
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
  // plinth:app-info (core 1.13, PRODUCTION.md D4): the data version the
  // hub page sent in `start`, carried the same way as the kv snapshot.
  const dataVersionStore = frameDataVersionStore(msg.dataVersion, (m) => send(m));
  // For tests: which app this frame runs.
  document.body.dataset.appId = parseManifest(manifestText).id;
  // The core and the app run in a dedicated Worker (`app-worker.js`). In the
  // bundle (plinth.js), `PLINTH_WORKER_SCRIPT` is the bundle itself as a
  // classic script; else the Worker imports app-worker.js next to this page.
  const bundleScript = typeof PLINTH_WORKER_SCRIPT === "string" ? PLINTH_WORKER_SCRIPT : null;
  app = new WorkerApp(startAppWorker(new URL("app-worker.js", document.baseURI).href, bundleScript));
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  // plinth:locale (core 1.13): the page gives its locale (the page can let
  // the user choose one for each app); else the frame's own.
  const locale = normalizeLocale(typeof msg.locale === "string" ? msg.locale : pageLocale());
  await app.load(new Uint8Array(msg.core), appWasm, {
    locale,
    log: (s) => console.log("[app]", s),
    manifestText,
    kvStore,
    dataVersionStore,
    kvQuota: msg.quota,
    refused: new Set(Array.isArray(msg.refused) ? msg.refused : []),
    askDialog: (kind, message) => ask("dialog", { kind, message: String(message) }),
    clipboard: {
      writeText: (text) => send({ type: "clipboard-write", text }),
      readText: () => ask("clipboard-read", {}),
    },
    // The parent page makes each request (it checks the capability again),
    // so a request carries the page's origin, not "null".
    netFetch: (url, method, headers, body) => ask("net-fetch", { url, method, headers, body }),
    // plinth:files: the page keeps the files (its IndexedDB, one namespace
    // for this app) and checks the capability and the path again.
    files: { call: (op, path, text) => ask(`files-${op}`, op === "write" ? { path, text } : { path }) },
    // plinth:secrets: an opaque origin has no IndexedDB, so the page keeps
    // the secrets (encrypted, WebCrypto) and checks the capability again. A
    // call over a limit is answered here: the page drops a message that is
    // too big.
    secrets: {
      call: (op, name, value) =>
        overLimit(op, name, value)
          ? Promise.resolve([false, 0, "", "denied:limit"])
          : ask(`secrets-${op}`, op === "set" ? { name, value } : op === "names" ? {} : { name }),
    },
    // store.sql: the page runs the database (SQLite in its own Worker, with
    // OPFS of the page origin) and checks the capability again.
    sql: { call: (kind, statement, params) => ask("sql-run", { kind, statement, params }) },
    // plinth:notify: an opaque origin cannot use the Notification API, so
    // the page shows the notification (it checks the capability again).
    notify: {
      show: (title, body, tag) => ask("notify-show", { title, body, tag }),
      cancel: (tag) => send({ type: "notify-cancel", tag }),
    },
    // Only the Hub app gets a `plinth:hub` snapshot; `FrameHub` runs on it
    // in the Worker, and its messages go to the page through this frame.
    hubSnapshot: msg.hub ?? null,
    send,
    ask,
  });
  renderer = new DomRenderer(tree, container, app, assets);
  renderer.setLocale(locale);
  // The app theme applies to this frame's app root only (the Hub page
  // around the frame keeps the host theme).
  renderer.setTheme(manifestText);
  await app.init([]);
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
  } else if (msg.type === "locale" && renderer && typeof msg.tag === "string" && (parentOrigin === "*" || event.origin === parentOrigin)) {
    renderer.setLocale(msg.tag);
  } else if (msg.type === "hub-state" && app && (parentOrigin === "*" || event.origin === parentOrigin)) {
    app.applyHubState(msg.state);
  }
});

if (embedded) send({ type: "ready" });
else showError("This page runs one app for the web App Hub. Open the hub (hub.html) to run an app.");
