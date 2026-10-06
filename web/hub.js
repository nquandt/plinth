// The web App Hub page (docs/web-hub.md): renders the registry's apps and
// an app page with the capability label, and opens an app in the web host.
// The logic is in `hub-logic.js`; this file only builds the DOM.
//
// Routes (hash, so the back button works and a link can name an app):
//   #/            the app list
//   #/app/<id>    one app
//   #/run/<id>    check the package, ask for consent, run the app
//   #/manage      the grants and the stored data of each app
// `?registry=<url>` reads another registry (default: this origin's root).
//
// Each app runs in `<iframe sandbox="allow-scripts">` (`app-frame.html`,
// opaque origin). This page owns the storage (`hub-storage.js`), the
// consent decisions (`hub-consent.js`) and the package checks
// (`hub-integrity.js`).

import {
  loadRegistry,
  loadAppDocument,
  latestVersion,
  filterApps,
  capabilityRows,
  hasUnsupported,
  openTarget,
  formatSize,
  riskWords,
  pickCore,
  packageUrl,
  coreUrl,
} from "./hub-logic.js";
import {
  manifestCapabilities,
  consentRows,
  needsConsent,
  effectiveGrants,
  recordDecisions,
  setGrant,
  decisionWords,
  emptyRecord,
} from "./hub-consent.js";
import { HubStore, memoryStorage, checkFrameMessage, applyKvMessage, startMessage, CHANNEL, KV_QUOTA } from "./hub-storage.js";
import { checkPackage, checkCore } from "./hub-integrity.js";

const main = document.getElementById("hub-main");
const status = document.getElementById("hub-status");
const registryLabel = document.getElementById("hub-registry");

let registry = null;
let query = "";

function openStorage(name) {
  try {
    const s = window[name];
    s.length; // throws if the browser blocks storage
    return s;
  } catch {
    return memoryStorage();
  }
}
const store = new HubStore(openStorage("localStorage"), openStorage("sessionStorage"));

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

function capitalize(s) {
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

function appHref(id) {
  return `#/app/${encodeURIComponent(id)}`;
}

function runHref(id) {
  return `#/run/${encodeURIComponent(id)}`;
}

function icon(app, large = false) {
  const cls = large ? "hub-icon hub-icon-large" : "hub-icon";
  if (app.icon) return el("img", { class: cls, src: app.icon, alt: "" });
  return el("span", { class: cls, "aria-hidden": "true" }, (app.name || "?").trim().charAt(0).toUpperCase());
}

function setView(...children) {
  main.replaceChildren(...children.filter((c) => c !== null && c !== undefined && c !== false));
}

/** Moves focus to the new view's heading, so a screen reader reads it. */
function focusHeading() {
  const h1 = main.querySelector("h1");
  if (h1) {
    h1.setAttribute("tabindex", "-1");
    h1.focus({ preventScroll: true });
  }
  window.scrollTo(0, 0);
}

function announce(text) {
  status.textContent = text;
}

// -- The app list --------------------------------------------------------------

function riskChips(app) {
  const rows = capabilityRows(app);
  if (rows.length === 0) return el("p", { class: "hub-meta" }, "No special capabilities");
  return el(
    "ul",
    { class: "hub-chips", "aria-label": "Capabilities" },
    rows.map((r) => el("li", { class: `hub-chip hub-risk-${r.risk}` }, r.description ? capitalize(r.description) : r.name)),
  );
}

function appCard(app) {
  const rows = capabilityRows(app);
  return el(
    "li",
    {},
    el(
      "a",
      { class: "hub-card", href: appHref(app.id) },
      icon(app),
      el(
        "div",
        { class: "hub-card-body" },
        el("h2", {}, app.name),
        el("p", { class: "hub-meta" }, `${app.publisher} · version ${app.latest}`),
        app.summary ? el("p", { class: "hub-summary" }, app.summary) : null,
        riskChips(app),
        hasUnsupported(rows) ? el("p", { class: "hub-meta hub-support-no" }, "Some features do not work in the browser") : null,
      ),
    ),
  );
}

function renderListItems(list, count) {
  const apps = filterApps(registry.apps, query);
  list.replaceChildren(...apps.map(appCard));
  const text =
    registry.apps.length === 0
      ? "This registry has no apps yet."
      : apps.length === registry.apps.length
        ? `${apps.length} ${apps.length === 1 ? "app" : "apps"}`
        : `${apps.length} of ${registry.apps.length} apps match “${query.trim()}”`;
  count.textContent = text;
  return text;
}

function showList() {
  document.title = "Plinth apps";
  const input = el("input", { class: "pl-input", id: "hub-q", type: "search", autocomplete: "off", spellcheck: "false", value: query });
  const count = el("p", { class: "hub-count", id: "hub-count" });
  const list = el("ul", { class: "hub-list", id: "hub-list", "aria-labelledby": "hub-title" });
  input.value = query;
  input.addEventListener("input", () => {
    query = input.value;
    announce(renderListItems(list, count));
  });
  setView(
    el("h1", { id: "hub-title" }, registry.name ?? "Apps"),
    registry.description ? el("p", {}, registry.description) : null,
    el("div", { class: "hub-search", role: "search" }, el("label", { for: "hub-q" }, "Search apps"), input),
    count,
    list,
  );
  renderListItems(list, count);
}

// -- One app ---------------------------------------------------------------------

function capabilityItem(row) {
  const support =
    row.support === "yes"
      ? el("p", {}, "Works in the browser. ", row.note)
      : row.support === "limited"
        ? el("p", {}, el("span", { class: "hub-support-limited" }, "Limited in the browser. "), row.note)
        : el("p", {}, el("span", { class: "hub-support-no" }, "Not supported in the browser. "), row.note);
  return el(
    "li",
    { class: "hub-cap" },
    el(
      "div",
      { class: "hub-cap-head" },
      el("p", { class: "hub-cap-title" }, row.description ? `${capitalize(row.description)}.` : row.name),
      el("span", { class: `hub-chip hub-risk-${row.risk}` }, riskWords(row.risk)),
    ),
    el("p", { class: "hub-code" }, row.name),
    row.rationale ? el("p", {}, `The app says: “${row.rationale}”`) : null,
    support,
  );
}

function versionsTable(doc) {
  const rows = (doc.versions ?? []).map((v) =>
    el(
      "tr",
      {},
      el("td", {}, v.version),
      el("td", {}, (v.published ?? "").slice(0, 10)),
      el("td", {}, formatSize(v.size)),
      el("td", {}, v.core ?? ""),
      el("td", {}, v.yanked ? `Withdrawn: ${v.yanked}` : v.signer ? "Signed" : "Not signed"),
    ),
  );
  return el(
    "div",
    { class: "hub-table-wrap", role: "region", "aria-label": "Version table", tabindex: "0" },
    el(
      "table",
      { class: "hub-table" },
      el("thead", {}, el("tr", {}, ["Version", "Published", "Size", "Core", "Status"].map((h) => el("th", { scope: "col" }, h)))),
      el("tbody", {}, rows),
    ),
  );
}

async function showApp(id) {
  const summary = registry.apps.find((a) => a.id.toLowerCase() === id.toLowerCase());
  if (!summary) {
    document.title = "App not found – Plinth apps";
    setView(el("a", { class: "hub-back", href: "#/" }, "← All apps"), el("h1", {}, "App not found"), el("p", {}, `This registry has no app with the id ${id}.`));
    return;
  }
  document.title = `${summary.name} – Plinth apps`;
  let doc = null;
  let docError = null;
  try {
    doc = await loadAppDocument(registry, summary.id);
  } catch (err) {
    docError = String(err?.message ?? err);
  }
  const version = doc ? latestVersion(doc) : null;
  const rows = capabilityRows(summary, version);
  const unsupported = rows.filter((r) => r.support === "no");
  const limited = rows.filter((r) => r.support === "limited");
  const target = openTarget(registry, version, `hub.html${appHref(summary.id)}`);

  const open = target.url
    ? el("a", { class: "hub-open", href: runHref(summary.id) }, `Open ${summary.name}`)
    : el("p", { class: "hub-note hub-note-danger" }, `Cannot open this app here. ${docError ?? target.reason}`);

  setView(
    el("a", { class: "hub-back", href: "#/" }, "← All apps"),
    el(
      "div",
      { class: "hub-app-head" },
      icon(summary, true),
      el(
        "div",
        {},
        el("h1", {}, summary.name),
        el("p", { class: "hub-meta" }, `${summary.publisher} · version ${version?.version ?? summary.latest}`),
        el("p", { class: "hub-meta hub-code" }, summary.id),
      ),
    ),
    doc?.description || summary.summary ? el("p", {}, doc?.description || summary.summary) : null,
    unsupported.length
      ? el(
          "p",
          { class: "hub-note hub-note-danger" },
          `The browser host does not support ${unsupported.map((r) => r.name).join(", ")}. The app opens, but these parts do not work here. Use the desktop Hub for them.`,
        )
      : null,
    !unsupported.length && limited.length
      ? el("p", { class: "hub-note" }, `Some capabilities work only partly in the browser: ${limited.map((r) => r.name).join(", ")}.`)
      : null,
    open,
    target.core ? el("p", { class: "hub-meta" }, `Runs in this browser with runtime core ${target.core}.`) : null,
    el(
      "section",
      { class: "hub-section", "aria-labelledby": "hub-caps" },
      el("h2", { id: "hub-caps" }, "What this app can do"),
      rows.length
        ? el("ul", { class: "hub-caps" }, rows.map(capabilityItem))
        : el("p", {}, "This app asks for no capabilities. It can only show its screens and keep time."),
    ),
    doc
      ? el(
          "section",
          { class: "hub-section", "aria-labelledby": "hub-versions" },
          el("h2", { id: "hub-versions" }, "Versions"),
          versionsTable(doc),
          doc.homepage ? el("p", {}, el("a", { href: doc.homepage, rel: "noopener noreferrer" }, "Home page")) : null,
        )
      : null,
  );
}

// -- Run an app: package checks, consent, the sandboxed frame -----------------

/** The app that runs now: `{ id, name, frame, declared, refused, prepared, sent }`. */
let session = null;
/** Increments on each route, so an old async view does not replace a newer one. */
let routeToken = 0;

function stopApp() {
  if (session) {
    session.frame.remove();
    session = null;
  }
  main.classList.remove("hub-main-wide");
  delete document.body.dataset.running;
  delete document.body.dataset.refused;
  delete document.body.dataset.consent;
  delete document.body.dataset.failed;
}

async function fetchBytes(url, what) {
  const res = await fetch(url, { cache: "no-cache" });
  if (!res.ok) throw new Error(`cannot download the ${what} (HTTP ${res.status})`);
  return new Uint8Array(await res.arrayBuffer());
}

/** Downloads and checks the package and the core of the latest version (docs/REGISTRY.md §5). */
async function preparePackage(summary) {
  const doc = await loadAppDocument(registry, summary.id);
  const version = latestVersion(doc);
  if (!version) throw new Error("the registry lists no version of this app");
  const core = pickCore(registry.cores, version.core);
  if (!core) throw new Error(`the registry has no runtime core ${version.core ?? ""} for this app`);
  const pkg = await fetchBytes(packageUrl(registry, version.sha256), "package");
  const checked = await checkPackage(pkg, version, summary.id);
  const coreBytes = await fetchBytes(coreUrl(registry, core.version), "runtime core");
  await checkCore(coreBytes, core);
  return { version, core, pkg, coreBytes, ...checked };
}

function signatureText(signature) {
  if (signature.status === "verified") return `Signed by ${signature.publisher}. The signature is correct.`;
  if (signature.status === "not-checked") return `Signed by ${signature.publisher}. Signature not checked: this browser cannot check Ed25519 signatures.`;
  return "Not signed. The package agrees with the registry digest.";
}

function capTitle(row) {
  return row.description ? `${capitalize(row.description)}.` : row.name;
}

async function runApp(id) {
  const token = routeToken;
  const summary = registry.apps.find((a) => a.id.toLowerCase() === id.toLowerCase());
  const back = el("a", { class: "hub-back", href: summary ? appHref(summary.id) : "#/" }, summary ? `← About ${summary.name}` : "← All apps");
  if (!summary) {
    document.title = "App not found – Plinth apps";
    setView(back, el("h1", {}, "App not found"), el("p", {}, `This registry has no app with the id ${id}.`));
    return;
  }
  document.title = `${summary.name} – Plinth apps`;
  setView(back, el("h1", {}, `Open ${summary.name}`), el("p", {}, "Checking the package…"));
  let prepared;
  try {
    prepared = await preparePackage(summary);
  } catch (err) {
    if (token !== routeToken) return;
    setView(
      back,
      el("h1", {}, `Cannot open ${summary.name}`),
      el("p", { class: "hub-note hub-note-danger", id: "hub-refused" }, `The page did not run this app: ${String(err?.message ?? err)}.`),
    );
    document.body.dataset.refused = "true";
    return;
  }
  if (token !== routeToken) return;
  const rows = consentRows(manifestCapabilities(prepared.manifestText), summary.labels, store.grants(summary.id), store.sessionGrants(summary.id));
  if (needsConsent(rows)) showConsent(summary, prepared, rows, back);
  else startFrame(summary, prepared, rows);
}

function consentItem(row, index) {
  const allow = row.decided ? row.allowed : row.defaultAllow;
  const radio = (value, label, checked) =>
    el("label", { class: "hub-choice" }, el("input", { type: "radio", name: `cap-${index}`, value, checked, "data-cap": row.name }), ` ${label}`);
  return el(
    "li",
    {},
    el(
      "fieldset",
      { class: "hub-cap" },
      el(
        "legend",
        { class: "hub-cap-legend" },
        el("span", { class: "hub-cap-title" }, capTitle(row)),
        " ",
        el("span", { class: `hub-chip hub-risk-${row.risk}` }, riskWords(row.risk)),
        row.isNew ? el("span", { class: "hub-chip hub-chip-new" }, "New in this version") : null,
      ),
      el("p", { class: "hub-code" }, row.name),
      row.rationale ? el("p", {}, `The app says: “${row.rationale}”`) : null,
      row.session ? el("p", { class: "hub-meta" }, "High risk: the decision lasts until you close the browser.") : null,
      el("div", { class: "hub-choices" }, radio("allow", "Allow", allow === true), radio("deny", "Deny", allow !== true)),
    ),
  );
}

function showConsent(summary, prepared, rows, back) {
  document.body.dataset.consent = summary.id;
  const asked = rows.filter((r) => r.risk !== "none");
  const hasNew = rows.some((r) => r.isNew);
  const form = el(
    "form",
    { class: "hub-consent", "aria-labelledby": "hub-consent-title" },
    el("ul", { class: "hub-caps" }, asked.map(consentItem)),
    el(
      "div",
      { class: "hub-actions" },
      el("button", { class: "hub-open hub-button", type: "submit", id: "hub-consent-ok" }, `Open ${summary.name}`),
      el("a", { class: "hub-back", href: appHref(summary.id) }, "Cancel"),
    ),
  );
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    const decisions = {};
    for (const input of form.querySelectorAll("input[type=radio]:checked")) decisions[input.dataset.cap] = input.value === "allow";
    const app = { id: summary.id, name: summary.name, version: prepared.version.version };
    const next = recordDecisions(store.grants(summary.id), store.sessionGrants(summary.id), app, rows, decisions);
    store.saveGrants(summary.id, next.record, next.sessionRecord);
    const after = consentRows(manifestCapabilities(prepared.manifestText), summary.labels, next.record, next.sessionRecord);
    delete document.body.dataset.consent;
    startFrame(summary, prepared, after);
    focusHeading();
  });
  setView(
    back,
    el("h1", { id: "hub-consent-title" }, `Allow ${summary.name} to use these capabilities?`),
    el("p", { class: "hub-meta" }, `${summary.publisher} · version ${prepared.version.version} · from ${registryLabel.textContent}`),
    el("p", {}, signatureText(prepared.signature)),
    hasNew
      ? el("p", { class: "hub-note" }, `Version ${prepared.version.version} asks for new capabilities. Decide about them before this version runs.`)
      : el(
          "p",
          {},
          "Decide for each capability. A denied capability does not stop the app: the app gets the answer “denied”. You can change these decisions later on the Manage page.",
        ),
    form,
  );
}

function allowed(s, name) {
  return s.declared.has(name) && !s.refused.has(name);
}

function startFrame(summary, prepared, rows) {
  const { refused } = effectiveGrants(rows);
  main.classList.add("hub-main-wide");
  // `allow-scripts` only: an opaque origin, no forms, no popups, no modal
  // dialogs, no top navigation, no same-origin access (docs/web-hub.md §4.1).
  const frame = el("iframe", { class: "hub-frame", sandbox: "allow-scripts", title: summary.name, src: "app-frame.html" });
  setView(
    el(
      "div",
      { class: "hub-run-bar" },
      el("a", { class: "hub-back", href: appHref(summary.id) }, `← About ${summary.name}`),
      el("h1", { class: "hub-run-title" }, summary.name),
      el("a", { class: "hub-back", href: "#/manage" }, "Manage"),
    ),
    el("p", { class: "hub-meta hub-run-note" }, signatureText(prepared.signature)),
    frame,
  );
  session = { id: summary.id, name: summary.name, frame, declared: new Set(rows.map((r) => r.name)), refused: new Set(refused), prepared, sent: false };
}

function answerFrame(s, id, value) {
  // The frame has an opaque origin: "*" is the only target origin that
  // reaches it. The message goes to the window of this frame only.
  s.frame.contentWindow?.postMessage({ channel: CHANNEL, type: "answer", id, value }, "*");
}

function onFrameMessage(event) {
  const s = session;
  if (!s || event.source !== s.frame.contentWindow) return;
  const msg = checkFrameMessage(event.data);
  if (!msg) return;
  if (msg.type === "kv-set" || msg.type === "kv-delete") {
    if (allowed(s, "store.kv")) applyKvMessage(store, s.id, msg, KV_QUOTA);
    return;
  }
  switch (msg.type) {
    case "ready": {
      if (s.sent) return;
      s.sent = true;
      const start = startMessage(store, s.id, { pkg: s.prepared.pkg.buffer, core: s.prepared.coreBytes.buffer, refused: s.refused });
      if (!allowed(s, "store.kv")) start.kv = {};
      s.frame.contentWindow.postMessage(start, "*", [start.pkg, start.core]);
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
      // The dialogs of the hub page: a sandboxed frame without
      // `allow-modals` cannot show them.
      let value = null;
      if (msg.kind === "alert") window.alert(msg.message);
      else if (msg.kind === "confirm") value = window.confirm(msg.message);
      else value = window.prompt(msg.message);
      answerFrame(s, msg.id, value);
      break;
    }
  }
}

window.addEventListener("message", onFrameMessage);

// -- Manage ------------------------------------------------------------------

function manageItem(app, index) {
  const summary = registry.apps.find((a) => a.id.toLowerCase() === app.id.toLowerCase());
  const record = app.record ?? emptyRecord(app.id, app.name);
  const caps = (record.capabilities ?? []).map((name) => ({ name, rationale: "" }));
  const rows = consentRows(caps, summary?.labels, record, store.sessionGrants(app.id));
  const headingId = `hub-manage-${index}`;
  const rerender = (text) => {
    showManage();
    const h = document.getElementById(headingId);
    if (h) {
      h.setAttribute("tabindex", "-1");
      h.focus();
    }
    announce(text);
  };
  const capItem = (row, i) => {
    const selectId = `${headingId}-cap-${i}`;
    const select = el(
      "select",
      { class: "pl-select hub-select", id: selectId, "data-cap": row.name },
      row.decided ? null : el("option", { value: "", selected: true, disabled: true }, "Not decided"),
      el("option", { value: "allow", selected: row.decided && row.allowed }, "Allow"),
      el("option", { value: "deny", selected: row.decided && !row.allowed }, "Deny"),
    );
    if (row.risk === "none") select.disabled = true;
    select.addEventListener("change", () => {
      const next = setGrant(store.grants(app.id) ?? record, store.sessionGrants(app.id), row.name, select.value === "allow");
      store.saveGrants(app.id, next.record, next.sessionRecord);
      rerender(`${app.name}: ${row.name} ${select.value === "allow" ? "allowed" : "denied"}.`);
    });
    return el(
      "li",
      { class: "hub-cap" },
      el(
        "div",
        { class: "hub-cap-head" },
        el("p", { class: "hub-cap-title" }, capTitle(row)),
        el("span", { class: `hub-chip hub-risk-${row.risk}` }, riskWords(row.risk)),
      ),
      el("p", { class: "hub-code" }, row.name),
      el("p", {}, decisionWords(row)),
      el("label", { class: "hub-select-label", for: selectId }, `Decision for ${row.name}`),
      select,
    );
  };
  const removeData = el("button", { class: "pl-button", type: "button", disabled: app.usage === 0 }, "Remove stored data");
  removeData.addEventListener("click", () => {
    store.kvClear(app.id);
    rerender(`The stored data of ${app.name} is removed.`);
  });
  const forget = el("button", { class: "pl-button", type: "button", disabled: !app.record }, "Forget decisions");
  forget.addEventListener("click", () => {
    store.forgetGrants(app.id);
    rerender(`The decisions for ${app.name} are removed. The app asks again when it opens.`);
  });
  return el(
    "section",
    { class: "hub-section hub-manage-app", "aria-labelledby": headingId, "data-app": app.id },
    el("h2", { id: headingId }, app.name),
    el("p", { class: "hub-code" }, app.id),
    el("p", {}, `Stored data: ${formatSize(app.usage)} of ${formatSize(KV_QUOTA)} (characters of keys and values).`),
    rows.length ? el("ul", { class: "hub-caps" }, rows.map(capItem)) : el("p", {}, "No capabilities."),
    el(
      "div",
      { class: "hub-actions" },
      removeData,
      forget,
      summary ? el("a", { class: "hub-back", href: runHref(summary.id) }, `Open ${app.name}`) : null,
    ),
  );
}

function showManage() {
  document.title = "Manage apps – Plinth apps";
  const apps = store.knownApps();
  setView(
    el("a", { class: "hub-back", href: "#/" }, "← All apps"),
    el("h1", {}, "Manage apps"),
    el("p", {}, "The decisions and the data of each app stay in this browser, for this site. Each app can read only its own data."),
    ...(apps.length ? apps.map(manageItem) : [el("p", { id: "hub-manage-empty" }, "No app has decisions or stored data yet.")]),
  );
}

// -- Routing -------------------------------------------------------------------

async function route() {
  const hash = location.hash || "#/";
  const token = ++routeToken;
  stopApp();
  const m = /^#\/(app|run)\/(.+)$/.exec(hash);
  if (m && m[1] === "app") await showApp(decodeURIComponent(m[2]));
  else if (m) await runApp(decodeURIComponent(m[2]));
  else if (hash === "#/manage") showManage();
  else showList();
  if (token !== routeToken) return; // a newer route replaced this view
  focusHeading();
  document.body.dataset.route = hash; // for tests: this view is rendered
}

async function start() {
  const param = new URLSearchParams(location.search).get("registry");
  const base = new URL(param ?? "/", location.href);
  if (!base.pathname.endsWith("/")) base.pathname += "/";
  try {
    registry = await loadRegistry(base.href);
  } catch (err) {
    setView(
      el("h1", {}, "Cannot read the registry"),
      el("p", { class: "hub-error" }, String(err?.message ?? err)),
      el("p", {}, "Start the server with: plinth registry serve <folder> --web"),
    );
    return;
  }
  registryLabel.textContent = base.origin === location.origin && base.pathname === "/" ? location.host : base.href;
  window.addEventListener("hashchange", () => route());
  await route();
  document.body.dataset.ready = "true";
}

start();
