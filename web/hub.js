// The web App Hub page (docs/web-hub.md): renders the registry's apps and
// an app page with the capability label, and opens an app in the web host.
// The logic is in `hub-logic.js`; this file only builds the DOM.
//
// Routes (hash, so the back button works and a link can name an app):
//   #/            the app list
//   #/app/<id>    one app
// `?registry=<url>` reads another registry (default: this origin's root).

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
} from "./hub-logic.js";

const main = document.getElementById("hub-main");
const status = document.getElementById("hub-status");
const registryLabel = document.getElementById("hub-registry");

let registry = null;
let query = "";

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
    ? el("a", { class: "hub-open", href: target.url }, `Open ${summary.name}`)
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

// -- Routing -------------------------------------------------------------------

async function route() {
  const hash = location.hash || "#/";
  const m = /^#\/app\/(.+)$/.exec(hash);
  if (m) await showApp(decodeURIComponent(m[1]));
  else showList();
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
