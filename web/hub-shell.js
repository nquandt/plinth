// The web App Hub shell (docs/web-hub.md): the host part of `hub.html`.
// There is one Hub UI, the Plinth app `examples/hub`; this file only
// bootstraps it and does the host work that the desktop host does in its
// own windows:
//
// 1. Reads the registry that serves this page and its static `hub.json`
//    (the Hub app id and the trusted Hub keys, written by
//    `plinth registry build --hub-trusted-key`), downloads the Hub app and
//    a core, and checks them (digest, signature). The Hub app gets
//    `hub.manage` only if a trusted key signed it (docs/HUB.md §4.1).
// 2. Runs the Hub app in this page with the `plinth:hub` backend
//    (`hub-host.js`, browse mode: the library is the registry listing; the
//    user's grants, blocks, pins and groups are in IndexedDB).
// 3. On a launch: the consent window (a host dialog, not app UI) for the
//    capabilities that are not decided, then the app window: a dialog with
//    the app in `<iframe sandbox="allow-scripts">` (`app-frame.html`, an
//    opaque origin). The page keeps the kv data of each app
//    (`hub-storage.js`) and answers the frame's clipboard and dialog
//    requests.

import { PlinthApp, readPlnt } from "./plinth-web.js";
import { Tree, DomRenderer } from "./dom-renderer.js";
import { loadRegistry, loadAppDocument, latestVersion, pickCore, packageUrl, coreUrl, fetchBytes } from "./registry-client.js";
import { checkPackage, checkCore } from "./hub-integrity.js";
import { HubHost, indexedDbPersist, memoryPersist } from "./hub-host.js";
import { HubStore, memoryStorage, checkFrameMessage, applyKvMessage, startMessage, CHANNEL, KV_QUOTA } from "./hub-storage.js";

const container = document.getElementById("app");
const status = document.getElementById("host-status");

/** `el("p", { class: "x" }, "text", child)`: a small DOM builder (text is never parsed as HTML). */
function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") node.className = v;
    else node.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

function announce(text) {
  status.textContent = text;
}

function capitalize(s) {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

function riskWords(risk) {
  return { none: "No risk", low: "Low risk", medium: "Medium risk", high: "High risk" }[risk] ?? "Unknown risk";
}

function showStartError(title, text) {
  document.title = "Plinth Hub";
  container.replaceChildren(
    el("main", { class: "host-error" }, el("h1", {}, title), el("p", {}, text), el("p", {}, "Start the demo with: bash scripts/web-hub-demo.sh")),
  );
  document.body.dataset.startError = text;
}

function openStorage(name) {
  try {
    const s = window[name];
    s.length; // throws if the browser blocks storage
    return s;
  } catch {
    return memoryStorage();
  }
}

/** The kv data of the launched apps, one namespace for each app. */
const appData = new HubStore(openStorage("localStorage"));

let host = null;
const coreCache = new Map(); // "<registry>|<version>" -> Uint8Array

/** The core that runs a version that needs core `needed`, from `registry`, checked against its digest. */
async function coreFor(registry, needed) {
  const core = pickCore(registry.cores, needed);
  if (!core) throw new Error(`the source has no runtime core ${needed ?? ""}`);
  const key = `${registry.base}|${core.version}`;
  if (!coreCache.has(key)) {
    const bytes = await fetchBytes(coreUrl(registry, core.version));
    await checkCore(bytes, core);
    coreCache.set(key, bytes);
  }
  return coreCache.get(key);
}

// -- Host dialogs --------------------------------------------------------------

/** Shows a short host message (for example "this app is blocked"). */
function notice(title, text) {
  const dialog = el(
    "dialog",
    { class: "host-dialog", "aria-labelledby": "host-notice-title" },
    el("h2", { id: "host-notice-title" }, title),
    el("p", {}, text),
    el("form", { method: "dialog", class: "host-actions" }, el("button", { class: "host-button host-primary", value: "ok" }, "OK")),
  );
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  document.body.dataset.notice = text;
}

/**
 * The consent window (docs/HUB.md §7.3): one Allow/Deny choice for each
 * capability that is not decided. Resolves with `{ name: allowed }`, or
 * null if the user cancels. Deny is the first choice for Medium and High
 * risk (they are not granted without a question).
 */
function askConsent(entry, version, rows) {
  return new Promise((resolve) => {
    const items = rows.map((row, i) =>
      el(
        "li",
        {},
        el(
          "fieldset",
          { class: "host-cap" },
          el(
            "legend",
            {},
            el("span", { class: "host-cap-title" }, row.description ? `${capitalize(row.description)}.` : row.name),
            " ",
            el("span", { class: `host-chip host-risk-${row.risk}` }, riskWords(row.risk)),
          ),
          el("p", { class: "host-code" }, row.name),
          row.rationale ? el("p", {}, `The app says: “${row.rationale}”`) : null,
          el(
            "div",
            { class: "host-choices" },
            el("label", {}, el("input", { type: "radio", name: `cap-${i}`, value: "allow", "data-cap": row.name }), " Allow"),
            el("label", {}, el("input", { type: "radio", name: `cap-${i}`, value: "deny", "data-cap": row.name, checked: true }), " Deny"),
          ),
        ),
      ),
    );
    const signer = version.signer ? `Signed by ${entry.publisher} (${version.signer}).` : `${entry.publisher || "Unknown publisher"} (unverified publisher).`;
    const form = el(
      "form",
      { method: "dialog" },
      el("ul", { class: "host-caps" }, items),
      el(
        "div",
        { class: "host-actions" },
        el("button", { class: "host-button host-primary", value: "open", id: "host-consent-open" }, `Open ${entry.name}`),
        el("button", { class: "host-button", value: "cancel", id: "host-consent-cancel" }, "Cancel"),
      ),
    );
    const dialog = el(
      "dialog",
      { class: "host-dialog", id: "host-consent", "aria-labelledby": "host-consent-title" },
      el("h2", { id: "host-consent-title" }, `Allow ${entry.name} to use these capabilities?`),
      el("p", { class: "host-meta" }, `Version ${version.version}. ${signer}`),
      el("p", {}, "A denied capability does not stop the app: the app gets the answer “denied”. You can change this later on the app page of the Hub."),
      form,
    );
    dialog.addEventListener("close", () => {
      const ok = dialog.returnValue === "open";
      const decisions = {};
      for (const input of form.querySelectorAll("input[type=radio]:checked")) decisions[input.dataset.cap] = input.value === "allow";
      dialog.remove();
      delete document.body.dataset.consent;
      resolve(ok ? decisions : null);
    });
    document.body.append(dialog);
    document.body.dataset.consent = entry.id;
    dialog.showModal();
  });
}

// -- The app window -------------------------------------------------------------

/** The app that runs now: `{ id, name, frame, dialog, declared, refused, pkg, core, sent }`. */
let session = null;

function closeApp() {
  if (!session) return;
  const s = session;
  session = null;
  s.frame.remove();
  if (s.dialog.open) s.dialog.close();
  s.dialog.remove();
  delete document.body.dataset.running;
}

function allowed(s, name) {
  return s.declared.includes(name) && !s.refused.includes(name);
}

function openAppWindow(entry, version, pkg, core) {
  closeApp();
  // `allow-scripts` only: an opaque origin, no forms, no popups, no modal
  // dialogs, no top navigation, no same-origin access (docs/web-hub.md §4).
  const frame = el("iframe", { class: "host-frame", sandbox: "allow-scripts", title: entry.name, src: "app-frame.html" });
  const close = el("button", { class: "host-button", type: "button", id: "host-app-close" }, "Close");
  const dialog = el(
    "dialog",
    { class: "host-window", id: "host-app", "aria-labelledby": "host-app-title" },
    el(
      "div",
      { class: "host-window-bar" },
      el("h2", { id: "host-app-title" }, entry.name),
      el("span", { class: "host-meta" }, `Version ${version.version}`),
      close,
    ),
    frame,
  );
  close.addEventListener("click", () => closeApp());
  dialog.addEventListener("close", () => closeApp());
  document.body.append(dialog);
  session = {
    id: entry.id,
    name: entry.name,
    frame,
    dialog,
    declared: version.capabilities,
    refused: host.refusedFor(entry.id, version.capabilities),
    pkg,
    core,
    sent: false,
  };
  dialog.showModal();
}

function answerFrame(s, id, value) {
  // The frame has an opaque origin: "*" is the only target origin that
  // reaches it. The message goes to the window of this frame only.
  s.frame.contentWindow?.postMessage({ channel: CHANNEL, type: "answer", id, value }, "*");
}

window.addEventListener("message", (event) => {
  const s = session;
  if (!s || event.source !== s.frame.contentWindow) return;
  const msg = checkFrameMessage(event.data);
  if (!msg) return;
  if (msg.type === "kv-set" || msg.type === "kv-delete") {
    if (allowed(s, "store.kv")) applyKvMessage(appData, s.id, msg, KV_QUOTA);
    return;
  }
  switch (msg.type) {
    case "ready": {
      if (s.sent) return;
      s.sent = true;
      const pkg = s.pkg.slice().buffer;
      const core = s.core.slice().buffer;
      const start = startMessage(appData, s.id, { pkg, core, refused: s.refused });
      if (!allowed(s, "store.kv")) start.kv = {};
      s.frame.contentWindow.postMessage(start, "*", [pkg, core]);
      break;
    }
    case "started":
      document.body.dataset.running = s.id;
      announce(`${s.name} is open.`);
      break;
    case "failed":
      document.body.dataset.failed = msg.message;
      break;
    case "clipboard-write":
      if (allowed(s, "clipboard.write")) navigator.clipboard?.writeText(msg.text).catch(() => {});
      break;
    case "clipboard-read":
      if (!allowed(s, "clipboard.read") || !navigator.clipboard?.readText) answerFrame(s, msg.id, null);
      else
        navigator.clipboard.readText().then(
          (text) => answerFrame(s, msg.id, text),
          () => answerFrame(s, msg.id, null),
        );
      break;
    case "dialog": {
      // The dialogs of this page: a sandboxed frame without `allow-modals`
      // cannot show them.
      let value = null;
      if (msg.kind === "alert") window.alert(msg.message);
      else if (msg.kind === "confirm") value = window.confirm(msg.message);
      else value = window.prompt(msg.message);
      answerFrame(s, msg.id, value);
      break;
    }
  }
});

/**
 * A launch from the Hub app (`plinth:hub` `launch`): the same plan as the
 * desktop host. A blocked app does not open. The newest version (or the
 * pinned one) asks for its undecided capabilities; if the user cancels,
 * the newest version whose capabilities are all decided runs.
 */
async function launchApp(id) {
  try {
    const entry = host.get(id);
    if (!entry) return notice("Cannot open the app", `${id} is not in the library.`);
    if (host.isBlocked(id)) return notice(`${entry.name} is blocked`, "A blocked app does not open. Unblock it on its page in the Hub.");
    let version = host.activeVersion(entry);
    const rows = host.consentRows(id, version);
    if (rows.length) {
      const decisions = await askConsent(entry, version, rows);
      if (decisions) host.recordConsent(id, version.version, decisions);
      else {
        version = host.runnableVersion(id);
        if (host.needsConsent(id, version.capabilities).length) return;
      }
    }
    const pkg = await host.packageBytes(id, version);
    const core = await coreFor(entry.registry, version.core);
    openAppWindow(entry, version, pkg, core);
  } catch (err) {
    console.error(err);
    notice("Cannot open the app", String(err?.message ?? err));
  }
}

// -- Start ---------------------------------------------------------------------

async function start() {
  const params = new URLSearchParams(location.search);
  const base = new URL(params.get("registry") ?? "/", location.href).href;
  // hub.json: a static file of the registry. Without it no key is trusted,
  // and the Hub app does not run (it needs hub.manage).
  let config = { trustedKeys: [], hub: "dev.plinth.hub" };
  try {
    const res = await fetch(new URL("hub.json", base), { cache: "no-cache" });
    if (res.ok) config = { ...config, ...(await res.json()) };
  } catch {
    // no config
  }

  let registry;
  try {
    registry = await loadRegistry(base);
  } catch (err) {
    return showStartError("Cannot read the registry", String(err?.message ?? err));
  }
  const summary = registry.apps.find((a) => a.id === config.hub);
  if (!summary) return showStartError("No Hub app", `The registry has no ${config.hub}. Build the registry with the Hub app in it.`);
  const version = latestVersion(await loadAppDocument(registry, summary.id));
  const core = pickCore(registry.cores, version?.core);
  if (!version || !core) return showStartError("Cannot run the Hub app", "The registry has no version of the Hub app or no runtime core for it.");

  let checked;
  let pkg;
  let coreBytes;
  try {
    pkg = await fetchBytes(packageUrl(registry, version.sha256));
    checked = await checkPackage(pkg, version, summary.id);
    coreBytes = await fetchBytes(coreUrl(registry, core.version));
    await checkCore(coreBytes, core);
  } catch (err) {
    return showStartError("The Hub app does not agree with the registry", String(err?.message ?? err));
  }
  // docs/HUB.md §4.1: hub.manage only for a package signed by a trusted key.
  const sig = checked.signature;
  if (sig.status !== "verified" || !config.trustedKeys.includes(sig.key)) {
    const why =
      sig.status === "not-checked"
        ? "This browser cannot check Ed25519 signatures, so it cannot trust the Hub app."
        : sig.status === "unsigned"
          ? "The Hub app is not signed."
          : `The Hub app is signed by ${sig.key}, which the registry's hub.json does not list as a trusted Hub key (plinth registry build --hub-trusted-key).`;
    return showStartError("The Hub app is not trusted", why);
  }

  let persist;
  try {
    persist = await indexedDbPersist();
  } catch {
    persist = memoryPersist(); // private mode: the library lasts for this page only
  }
  host = await new HubHost({ persist, sources: [[registry.name ?? location.host, base]], hide: [summary.id], onLaunch: (id) => launchApp(id) }).load();

  const { appWasm, manifestText, assets } = await readPlnt(pkg);
  const app = new PlinthApp();
  const tree = new Tree();
  app.onCommit = (ops) => tree.apply(ops);
  await app.load(coreBytes, appWasm, { log: (s) => console.log("[hub]", s), manifestText, hub: host });
  container.replaceChildren();
  new DomRenderer(tree, container, app, assets);
  app.init([]);
  document.title = summary.name;
  document.body.dataset.ready = "true";
}

start().catch((err) => {
  console.error(err);
  showStartError("The Hub did not start", String(err?.stack ?? err));
});
