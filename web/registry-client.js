// A registry client for the web host (docs/REGISTRY.md): reads the service
// index, the app list, app documents and the core list, searches a static
// registry, and builds package and core URLs. The web Hub host
// (`hub-host.js`) uses it for `plinth:hub` search, install and updates, and
// `hub-shell.js` uses it to find the Hub app and the core. No DOM code.

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
  if (schema !== "plinth.registry/1") throw new Error(`${url}: this host does not know the registry schema "${schema}"`);
}

/**
 * Reads the service index, the app list (all pages) and the core list of
 * the registry at `base` (a URL; a missing final `/` is added). App icons
 * become absolute URLs.
 */
export async function loadRegistry(base, fetchImpl = globalThis.fetch) {
  const root = new URL(base);
  if (!root.pathname.endsWith("/")) root.pathname += "/";
  const indexUrl = new URL(SERVICE_INDEX, root).href;
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
    try {
      cores = (await getJson(resolveTemplate(indexUrl, resources.cores), fetchImpl)).cores ?? [];
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
  return doc;
}

/** The newest version that is not yanked (docs/REGISTRY.md §4: a yanked version is not installed for new users). */
export function latestVersion(doc) {
  return (doc?.versions ?? []).find((v) => !v.yanked) ?? null;
}

/**
 * The apps of a static registry that match `query`: the id, the name, the
 * summary or a category contains it (no case). The same rule as
 * `plinth_registry::source::Registry::search`. An empty query lists every app.
 */
export function searchApps(apps, query) {
  const needle = String(query ?? "").toLowerCase();
  return apps.filter(
    (a) =>
      a.id.toLowerCase().includes(needle) ||
      a.name.toLowerCase().includes(needle) ||
      (a.summary ?? "").toLowerCase().includes(needle) ||
      (a.categories ?? []).some((c) => c.toLowerCase().includes(needle)),
  );
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

/** Downloads a file as bytes. */
export async function fetchBytes(url, fetchImpl = globalThis.fetch) {
  const res = await fetchImpl(url, { cache: "no-cache" });
  if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
  return new Uint8Array(await res.arrayBuffer());
}
