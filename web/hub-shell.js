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
// 2. Runs the Hub app in a sandboxed frame, as every app (SPEC.md §10.3),
//    with the `plinth:hub` backend (`hub-host.js`, browse mode: the library
//    is the registry listing; the user's grants, blocks, pins and groups
//    are in IndexedDB of this page). The frame runs `plinth:hub` on a copy
//    (`FrameHub`) that this page keeps up to date (`frame-host.js`).
// 3. On a launch: the consent window (a host dialog, not app UI) for the
//    capabilities that are not decided, then the app window: a dialog with
//    the app in its sandboxed frame. The frame pair (`app-frame.js` in the
//    frame, `frame-host.js` here) is the same for every page that shows a
//    Plinth app; this page adds only the Hub parts.

import { loadRegistry, loadAppDocument, latestVersion, pickCore, packageUrl, coreUrl, fetchBytes } from "./registry-client.js";
import { checkPackage, checkCore } from "./hub-integrity.js";
import { HubHost, indexedDbPersist, memoryPersist } from "./hub-host.js";
import { HubStore, memoryStorage } from "./hub-storage.js";
import { AppFrame } from "./frame-host.js";

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
      el("h2", { id: "host-consent-title" }, `Allow ${entry.name} to do these things?`),
      el("p", { class: "host-meta" }, `Version ${version.version}. ${signer}`),
      el("p", {}, "If you do not allow something, the app still opens, but that part does not work. You can change this later on the app page in the Hub."),
      form,
    );
    // The answer comes from the button (the submit event), not only from
    // the close event: a close event can come late or not at all in some
    // browsers. Escape (cancel) or a close without a button is Cancel.
    let done = false;
    const finish = (ok) => {
      if (done) return;
      done = true;
      const decisions = {};
      for (const input of form.querySelectorAll("input[type=radio]:checked")) decisions[input.dataset.cap] = input.value === "allow";
      if (dialog.open) dialog.close();
      dialog.remove();
      delete document.body.dataset.consent;
      resolve(ok ? decisions : null);
    };
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      finish(event.submitter?.value === "open");
    });
    dialog.addEventListener("cancel", () => finish(false));
    dialog.addEventListener("close", () => finish(dialog.returnValue === "open"));
    document.body.append(dialog);
    document.body.dataset.consent = entry.id;
    dialog.showModal();
  });
}

// -- The app window -------------------------------------------------------------

/** The app that runs now: `{ id, name, frame (AppFrame), dialog }`. */
let session = null;

function closeApp() {
  if (!session) return;
  const s = session;
  session = null;
  s.frame.destroy();
  s.dialog.remove();
  container.inert = false;
  delete document.body.dataset.running;
  if (s.returnFocus?.isConnected) s.returnFocus.focus();
}

/** The host's app window: a modal dialog with the app in its sandboxed frame (`frame-host.js`). */
function openAppWindow(entry, version, pkg, core) {
  closeApp();
  const frame = new AppFrame({
    appId: entry.id,
    title: entry.name,
    pkg,
    core,
    declared: version.capabilities,
    refused: host.refusedFor(entry.id, version.capabilities),
    store: appData,
    onStarted: () => {
      document.body.dataset.running = entry.id;
      announce(`${entry.name} is open.`);
    },
    onFailed: (message) => {
      document.body.dataset.failed = message;
    },
  });
  const close = el("button", { class: "host-button", type: "button", id: "host-app-close" }, "Close");
  // A fixed overlay with role="dialog", not a modal <dialog>: headless
  // Chromium does not route mouse input to an out-of-process iframe in the
  // top layer. The Hub app behind it is inert while it is open.
  const dialog = el(
    "div",
    { class: "host-window", id: "host-app", role: "dialog", "aria-modal": "true", "aria-labelledby": "host-app-title" },
    el(
      "div",
      { class: "host-window-bar" },
      el("h2", { id: "host-app-title" }, entry.name),
      el("span", { class: "host-meta" }, `Version ${version.version}`),
      close,
    ),
    frame.element,
  );
  close.addEventListener("click", () => closeApp());
  // Escape in the window bar closes it (a key in the app goes to the frame).
  dialog.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && session?.dialog === dialog) closeApp();
  });
  const returnFocus = document.activeElement;
  document.body.append(dialog);
  container.inert = true;
  session = { id: entry.id, frame, dialog, returnFocus };
  frame.element.focus();
}

/**
 * A launch from the Hub app (`plinth:hub` `launch`): the same plan as the
 * desktop host. A blocked app does not open. The newest version (or the
 * pinned one) asks for its undecided capabilities; if the user cancels,
 * the newest version whose capabilities are all decided runs.
 */
let launching = false;

async function launchApp(id) {
  // One launch at a time: a second Open while the consent window or a
  // download is open does nothing.
  document.body.dataset.launchCalls = String(Number(document.body.dataset.launchCalls ?? 0) + 1); // for tests
  if (launching) return;
  launching = true;
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
  } finally {
    launching = false;
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

  const hubFrame = new AppFrame({
    appId: summary.id,
    title: summary.name,
    pkg,
    core: coreBytes,
    declared: version.capabilities.map((c) => c.name ?? c),
    refused: [],
    store: appData,
    hub: host,
    onStarted: () => {
      document.body.dataset.ready = "true";
    },
    onFailed: (message) => showStartError("The Hub app did not start", message),
  });
  hubFrame.element.id = "hub-frame";
  container.replaceChildren(hubFrame.element);
  document.title = summary.name;
}

start().catch((err) => {
  console.error(err);
  showStartError("The Hub did not start", String(err?.stack ?? err));
});
