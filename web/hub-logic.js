// The web App Hub's logic (docs/web-hub.md): reads a registry
// (docs/REGISTRY.md), filters its apps, explains each capability for the
// browser host, and builds the URL that opens an app in the web host
// (`index.html`). No DOM code, so the node test (`test/run-hub-logic.mjs`)
// runs it directly; `hub.js` renders it.

/** The service index file name (docs/REGISTRY.md §2). */
export const SERVICE_INDEX = "plinth-registry.json";

/** Fills a URL template (`apps/{id}/index.json`) and resolves it against `base`. */
export function resolveTemplate(base, template, values = {}) {
  const filled = template.replace(/\{(\w+)\}/g, (_, key) => encodeURIComponent(values[key] ?? ""));
  return new URL(filled, base).href;
}

async function getJson(url, fetchImpl) {
  const res = await fetchImpl(url, { cache: "no-cache" });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return res.json();
}

function checkSchema(doc, url) {
  const schema = doc?.schema ?? "";
  if (schema !== "plinth.registry/1") throw new Error(`${url}: this page does not know the registry schema "${schema}"`);
}

/**
 * Reads the service index, the app list (all pages) and the core list of
 * the registry at `base` (a URL that ends with `/`). App icons become
 * absolute URLs.
 */
export async function loadRegistry(base, fetchImpl = globalThis.fetch) {
  const indexUrl = new URL(SERVICE_INDEX, base).href;
  const index = await getJson(indexUrl, fetchImpl);
  checkSchema(index, indexUrl);
  const resources = index.resources ?? {};
  if (!resources.apps || !resources.app || !resources.package) throw new Error(`${indexUrl}: "resources" needs apps, app and package`);

  const apps = [];
  let pageUrl = resolveTemplate(indexUrl, resources.apps);
  for (let pages = 0; pageUrl && pages < 100; pages++) {
    const page = await getJson(pageUrl, fetchImpl);
    checkSchema(page, pageUrl);
    for (const app of page.apps ?? []) {
      apps.push({ ...app, icon: app.icon ? new URL(app.icon, pageUrl).href : null });
    }
    pageUrl = page.next ? new URL(page.next, pageUrl).href : null;
  }
  apps.sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));

  let cores = [];
  if (resources.cores) {
    const coresUrl = resolveTemplate(indexUrl, resources.cores);
    try {
      const doc = await getJson(coresUrl, fetchImpl);
      cores = doc.cores ?? [];
    } catch {
      cores = []; // optional resource (docs/REGISTRY.md §2)
    }
  }
  return { base: indexUrl, name: index.name ?? null, description: index.description ?? null, resources, apps, cores };
}

/** The app document of `id` (docs/REGISTRY.md §4). */
export async function loadAppDocument(registry, id, fetchImpl = globalThis.fetch) {
  const url = resolveTemplate(registry.base, registry.resources.app, { id });
  const doc = await getJson(url, fetchImpl);
  checkSchema(doc, url);
  if (doc.icon) doc.icon = new URL(doc.icon, url).href;
  return doc;
}

/** The newest version that is not yanked, else the newest one. */
export function latestVersion(doc) {
  const versions = doc?.versions ?? [];
  return versions.find((v) => !v.yanked) ?? versions[0] ?? null;
}

/** The apps whose name, id, publisher, summary or capabilities contain every word of `query`. */
export function filterApps(apps, query) {
  const words = String(query ?? "").toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return apps;
  return apps.filter((app) => {
    const text = [app.name, app.id, app.publisher, app.summary ?? "", ...(app.capabilities ?? [])].join(" ").toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

function parseCoreVersion(v) {
  const m = /^(\d+)\.(\d+)$/.exec(String(v ?? ""));
  return m ? [Number(m[1]), Number(m[2])] : null;
}

/**
 * The core to run an app that needs core `needed` ("1.9"): the newest core
 * with the same major version and a minor version that is not lower
 * (SPEC.md §10.5). `null` if the registry has none.
 */
export function pickCore(cores, needed) {
  const want = parseCoreVersion(needed);
  const fits = (cores ?? [])
    .map((c) => ({ core: c, v: parseCoreVersion(c.version) }))
    .filter(({ v }) => v && (!want || (v[0] === want[0] && v[1] >= want[1])))
    .sort((a, b) => b.v[0] - a.v[0] || b.v[1] - a.v[1]);
  return fits.length ? fits[0].core : null;
}

/** The URL of a package file by digest. */
export function packageUrl(registry, sha256) {
  return resolveTemplate(registry.base, registry.resources.package, { sha256 });
}

/** The URL of a core file, or null if the registry serves no cores. */
export function coreUrl(registry, version) {
  return registry.resources.core ? resolveTemplate(registry.base, registry.resources.core, { version }) : null;
}

/**
 * What the browser host does with a capability (`plinth-web.js`). This is
 * a fact about the web host, not about the capability: the risk level and
 * the description come from the registry (`labels`, made from the shared
 * capability map in `crates/plinth-link/src/capabilities.rs`).
 *
 * `support` is "yes", "limited" or "no".
 */
export function browserSupport(name) {
  if (name === "store.kv") return { support: "yes", note: "Data stays in this browser, for this site." };
  if (name === "clipboard.write") return { support: "yes", note: "" };
  if (name === "clipboard.read")
    return { support: "limited", note: "The browser asks for permission. The first read can return nothing." };
  if (name === "net.local" || name.startsWith("net:"))
    return { support: "limited", note: "The server must allow requests from this page (CORS). Other requests fail." };
  if (name === "hub.manage") return { support: "no", note: "Only the desktop Hub can manage apps." };
  return { support: "no", note: "The browser host does not know this capability. Calls to it are refused." };
}

/**
 * The capability label of an app for this page: one row for each
 * capability, with the risk level and description from the registry and the
 * browser support. `summary` is an app-list entry; `version` (optional) is
 * an app-document version, used for its reasons when the list has no labels.
 */
export function capabilityRows(summary, version = null) {
  const labels = summary?.labels ?? [];
  const names = labels.length ? labels.map((l) => l.name) : (version?.capabilities ?? []).map((c) => c.name);
  const all = names.length ? names : summary?.capabilities ?? [];
  return all.map((name) => {
    const label = labels.find((l) => l.name === name);
    const rationale = label?.rationale || (version?.capabilities ?? []).find((c) => c.name === name)?.rationale || "";
    return {
      name,
      risk: label?.risk ?? "unknown",
      description: label?.description ?? "",
      rationale,
      ...browserSupport(name),
    };
  });
}

/** True if the browser host cannot give at least one of the app's capabilities. */
export function hasUnsupported(rows) {
  return rows.some((r) => r.support === "no");
}

/**
 * The web host URL that runs `version` of an app (`index.html?app=…&core=…`),
 * relative to the hub page, or null with a reason when it cannot run.
 */
export function openTarget(registry, version, back = "") {
  if (!version) return { url: null, reason: "The registry lists no version of this app." };
  const core = pickCore(registry.cores, version.core);
  if (!core) {
    const needed = version.core ? `core ${version.core}` : "core";
    return { url: null, reason: `The registry has no runtime ${needed} for this app. Build the registry with --with-core.` };
  }
  const params = new URLSearchParams({ app: packageUrl(registry, version.sha256), core: coreUrl(registry, core.version) });
  if (back) params.set("back", back);
  return { url: `index.html?${params}`, reason: null, core: core.version };
}

/** A size in bytes as a short text ("2.1 KB"). */
export function formatSize(bytes) {
  if (!Number.isFinite(bytes)) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/** The words for a risk level (docs/HUB.md §7.2). */
export function riskWords(risk) {
  return { none: "No risk", low: "Low risk", medium: "Medium risk", high: "High risk" }[risk] ?? "Unknown risk";
}
