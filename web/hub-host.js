// The `plinth:hub` backend of the web host (docs/web-hub.md, docs/HUB.md
// §4.1, §12.2): the library, grants, blocks, groups, pins, sources and
// updates of the Hub, for the Hub app (`examples/hub`) that runs in the
// browser. It is the web twin of `HubService` in `crates/plinth-hub`: the
// JSON of `listApps`, `appInfo`, `search` and `checkUpdates` has the same
// fields, and the rules (risk-level defaults, re-consent on update,
// signer checks, publisher blocks) are the same.
//
// `plinth:hub` calls are synchronous in the app. Thus the host keeps the
// whole state in memory and writes it to `persist` (IndexedDB in the
// browser, `indexedDbPersist`) after each change, in order. Package files
// are kept by digest in the same store. Network calls (`search`,
// `install`, `checkUpdates`, `update`) are async and use `fetch` against
// registries (`registry-client.js`); each package is checked
// (`hub-integrity.js`) before it is added.
//
// No DOM code: `test/run-hub-host.mjs` runs it in Node.

import { loadRegistry, loadAppDocument, latestVersion, searchApps, pickCore, packageUrl, fetchBytes } from "./registry-client.js";
import { checkPackage, sha256Hex } from "./hub-integrity.js";

/**
 * The capability map (`crates/plinth-link/src/capabilities.rs`): risk level
 * and description. A name that is not in it is "medium" with no
 * description, the same rule as `plinth_hub::risk_of` ("safer to ask").
 */
const CAPABILITIES = {
  "store.kv": { risk: "low", description: "save data on this device" },
  "clipboard.write": { risk: "low", description: "write to the clipboard" },
  "clipboard.read": { risk: "medium", description: "read the clipboard" },
  "net.local": { risk: "medium", description: "connect to devices on your local network" },
  "hub.manage": { risk: "high", description: "manage the Hub library: list, launch, grant and block apps" },
};

export function riskOf(name) {
  return Object.hasOwn(CAPABILITIES, name) ? CAPABILITIES[name].risk : "medium";
}

export function descriptionOf(name) {
  return Object.hasOwn(CAPABILITIES, name) ? CAPABILITIES[name].description : "";
}

/** None and Low are granted at install, with no question (docs/HUB.md §7.2). */
export function isLowRisk(name) {
  const r = riskOf(name);
  return r === "none" || r === "low";
}

function semverKey(v) {
  const parts = String(v).split(/[.\-+]/).map((p) => (/^\d+$/.test(p) ? Number(p) : 0));
  return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
}

/** True if version `a` is newer than version `b` (semver order, as `plinth_hub::is_newer`). */
export function isNewer(a, b) {
  const x = semverKey(a);
  const y = semverKey(b);
  for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] > y[i];
  return false;
}

function emptyState() {
  return { schema: "plinth.web-hub-state/1", apps: {}, groups: [], grants: {}, blocks: { apps: [], publishers: [] }, updates: {}, sources: {} };
}

function message(err) {
  return String(err?.message ?? err);
}

// -- Persistence ---------------------------------------------------------------

/** A store in memory (tests, and a fallback when IndexedDB is not there). */
export function memoryPersist() {
  const files = new Map();
  let state = null;
  return {
    async load() {
      return state === null ? null : JSON.parse(state);
    },
    async save(value) {
      state = JSON.stringify(value);
    },
    async getPackage(digest) {
      return files.get(digest) ?? null;
    },
    async putPackage(digest, bytes) {
      files.set(digest, bytes.slice());
    },
  };
}

/** The browser store: one IndexedDB database with the state and the package files. */
export async function indexedDbPersist(name = "plinth-web-hub") {
  const db = await new Promise((resolve, reject) => {
    const req = indexedDB.open(name, 1);
    req.onupgradeneeded = () => req.result.createObjectStore("files");
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
  const run = (mode, op) =>
    new Promise((resolve, reject) => {
      const tx = db.transaction("files", mode);
      const req = op(tx.objectStore("files"));
      tx.oncomplete = () => resolve(req.result);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  return {
    async load() {
      return (await run("readonly", (s) => s.get("state"))) ?? null;
    },
    async save(value) {
      await run("readwrite", (s) => s.put(value, "state"));
    },
    async getPackage(digest) {
      const v = await run("readonly", (s) => s.get(`package:${digest}`));
      return v ? new Uint8Array(v) : null;
    },
    async putPackage(digest, bytes) {
      await run("readwrite", (s) => s.put(bytes.slice().buffer, `package:${digest}`));
    },
  };
}

// -- The Hub -----------------------------------------------------------------

export class HubHost {
  /**
   * `persist`: `memoryPersist()` or `indexedDbPersist()`.
   * `onLaunch(id)`: the shell opens the app (`launch` returns at once).
   * `fetchImpl`: `fetch` (tests can count requests).
   */
  constructor({ persist = memoryPersist(), onLaunch = () => {}, fetchImpl = globalThis.fetch, now = () => Math.floor(Date.now() / 1000) } = {}) {
    this.persist = persist;
    this.onLaunch = onLaunch;
    this.fetchImpl = (...args) => fetchImpl(...args);
    this.now = now;
    this.state = emptyState();
    this.saving = Promise.resolve();
  }

  /** Reads the stored state. Call it one time, before the Hub app starts. */
  async load() {
    const stored = await this.persist.load();
    this.state = { ...emptyState(), ...(stored ?? {}) };
    return this;
  }

  /** Writes the state; the writes run in order. */
  save() {
    const snapshot = structuredClone(this.state);
    this.saving = this.saving.then(() => this.persist.save(snapshot)).catch((err) => console.error("the Hub cannot save its state:", err));
    return this.saving;
  }

  /** Resolves when every write so far is done. */
  flush() {
    return this.saving;
  }

  // -- Sources (docs/REGISTRY.md §9) ------------------------------------------

  /** Adds a source (or changes its base). The shell adds the registry that serves the page. */
  addSource(name, base) {
    if (this.state.sources[name] === base) return;
    this.state.sources[name] = base;
    this.save();
  }

  sources() {
    return Object.entries(this.state.sources).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  }

  // -- The library -----------------------------------------------------------

  /** The library entries, sorted by id (as `Hub::list`). */
  list() {
    return Object.values(this.state.apps).sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  }

  get(id) {
    return Object.hasOwn(this.state.apps, id) ? this.state.apps[id] : null;
  }

  mustGet(id) {
    const entry = this.get(id);
    if (!entry) throw new Error(`${id} is not in the library`);
    return entry;
  }

  activeVersion(entry) {
    if (entry.pinned) {
      const v = entry.versions.find((x) => x.version === entry.pinned);
      if (v) return v;
    }
    return entry.versions.at(-1) ?? null;
  }

  newestInstalled(id) {
    const entry = this.get(id);
    if (!entry || entry.versions.length === 0) return null;
    return entry.versions.reduce((best, v) => (isNewer(v.version, best.version) ? v : best));
  }

  grants(id) {
    return this.state.grants[id] ?? {};
  }

  isAppBlocked(id) {
    return this.state.blocks.apps.includes(id);
  }

  isPublisherBlocked(key) {
    return this.state.blocks.publishers.includes(key);
  }

  isPublisherOfBlocked(id) {
    const entry = this.get(id);
    if (!entry || this.state.blocks.publishers.length === 0) return false;
    return entry.versions.some((v) => v.signer && this.isPublisherBlocked(v.signer));
  }

  /** Whether `id` must not run: the app or the publisher of one of its versions is blocked. */
  isBlocked(id) {
    return this.isAppBlocked(id) || this.isPublisherOfBlocked(id);
  }

  setGrantInner(id, capability, allowed, versionSeen, byDefault) {
    this.state.grants[id] = { ...this.grants(id), [capability]: { decision: allowed ? "allowed" : "refused", time: this.now(), versionSeen, byDefault } };
  }

  /** The declared capabilities that need a decision: Medium/High with no grant (`Hub::needs_consent`). */
  needsConsent(id, declared) {
    const grants = this.grants(id);
    return declared.filter((c) => !grants[c] && !isLowRisk(c));
  }

  /** The version to run: the pinned one, else the newest one whose capabilities are all decided, else the newest (`Hub::runnable_version`). */
  runnableVersion(id) {
    const entry = this.mustGet(id);
    if (entry.pinned) {
      const v = entry.versions.find((x) => x.version === entry.pinned);
      if (v) return v;
    }
    for (const v of [...entry.versions].reverse()) {
      if (this.needsConsent(id, v.capabilities).length === 0) return v;
    }
    const last = entry.versions.at(-1);
    if (!last) throw new Error(`${id} has no versions`);
    return last;
  }

  /**
   * The capabilities that the app may not use (`Hub::policy_for`): a refused
   * grant, or no grant for a Medium/High capability. `hub.manage` is never
   * given to a launched app: only the trusted Hub app gets it (docs/HUB.md §4.1).
   */
  refusedFor(id, declared) {
    const grants = this.grants(id);
    return declared.filter((c) => c === "hub.manage" || (grants[c] ? grants[c].decision !== "allowed" : !isLowRisk(c)));
  }

  /** The rows of the consent window for `version` of `id`: the undecided capabilities with their label. */
  consentRows(id, version) {
    return this.needsConsent(id, version.capabilities).map((name) => ({
      name,
      risk: riskOf(name),
      description: descriptionOf(name),
      rationale: version.rationales?.[name] ?? "",
    }));
  }

  /** Records the decisions of the consent window (`decisions`: name -> allowed). */
  recordConsent(id, version, decisions) {
    for (const [name, allowed] of Object.entries(decisions)) this.setGrantInner(id, name, Boolean(allowed), version, false);
    this.save();
  }

  /** The bytes of an installed version, checked against its digest again. */
  async packageBytes(version) {
    const bytes = await this.persist.getPackage(version.digest);
    if (!bytes) throw new Error(`the package of version ${version.version} is not in this browser any more`);
    if ((await sha256Hex(bytes)) !== version.digest) throw new Error(`the stored package of version ${version.version} changed`);
    return bytes;
  }

  /**
   * Checks and adds a package from a registry (`Hub::add_package`): the
   * digest, the manifest and the signature (`checkPackage`), a core that
   * can run it, the blocks, and the signer of the previous version. Grants
   * the None/Low capabilities by default. Returns the app id.
   */
  async addPackage(bytes, registryVersion, appId, registry) {
    const { manifest, signature } = await checkPackage(bytes, registryVersion, appId);
    if (!pickCore(registry.cores, registryVersion.core)) {
      throw new Error(`the source has no runtime core ${registryVersion.core ?? ""} that can run ${manifest.id}`);
    }
    if (this.isBlocked(manifest.id)) throw new Error(`${manifest.id} is blocked`);
    const signer = signature.key;
    if (signer && this.isPublisherBlocked(signer)) throw new Error(`publisher ${signature.publisher} (${signer}) is blocked`);
    const digest = registryVersion.sha256.toLowerCase();
    await this.persist.putPackage(digest, bytes);
    const entry =
      this.get(manifest.id) ??
      (this.state.apps[manifest.id] = { id: manifest.id, name: manifest.name, versions: [], pinned: null, groups: [], added: this.now(), source: "file", registry: null });
    const previous = entry.versions.at(-1);
    if (previous?.signer && signer && previous.signer !== signer) {
      throw new Error(`${manifest.id} is signed by a different key (${signer}) than the installed version (${previous.signer}); key rotation is not supported yet`);
    }
    entry.name = manifest.name || entry.name;
    const declared = manifest.capabilities.map((c) => c.name);
    if (!entry.versions.some((v) => v.version === manifest.version)) {
      entry.versions.push({
        version: manifest.version,
        digest,
        capabilities: declared,
        signer,
        // Kept so that `appInfo` needs no package read: the manifest's
        // publisher and reasons, and the core that the version needs.
        publisher: manifest.publisher,
        rationales: Object.fromEntries(manifest.capabilities.map((c) => [c.name, c.rationale])),
        core: registryVersion.core ?? null,
      });
    }
    const grants = this.grants(manifest.id);
    for (const cap of declared) if (!grants[cap] && isLowRisk(cap)) this.setGrantInner(manifest.id, cap, true, manifest.version, true);
    this.save();
    return manifest.id;
  }

  // -- `plinth:hub`: synchronous calls ----------------------------------------
  // Each one throws an Error on failure; `plinth-web.js` answers "denied".

  /** One library app as the JSON object of `listApps`/`appInfo` (`HubService::app_json`), or null with no versions. */
  appJson(entry) {
    const active = this.activeVersion(entry);
    if (!active) return null;
    const grants = this.grants(entry.id);
    const update = this.state.updates[entry.id] ?? null;
    const versions = [...entry.versions].sort((a, b) => (isNewer(a.version, b.version) ? -1 : isNewer(b.version, a.version) ? 1 : 0));
    return {
      id: entry.id,
      name: entry.name,
      version: active.version,
      publisher: active.publisher ?? "",
      signer: active.signer ?? "",
      source: entry.registry?.name ?? "",
      blocked: this.isAppBlocked(entry.id),
      publisherBlocked: this.isPublisherOfBlocked(entry.id),
      groups: entry.groups,
      capabilities: active.capabilities.map((name) => ({
        name,
        risk: riskOf(name),
        description: descriptionOf(name),
        rationale: active.rationales?.[name] ?? "",
        decided: Boolean(grants[name]),
        allowed: grants[name]?.decision === "allowed",
        byDefault: Boolean(grants[name]?.byDefault),
      })),
      pinned: entry.pinned ?? "",
      versions: versions.map((v) => ({ version: v.version, signer: v.signer ?? "", capabilities: v.capabilities })),
      update: update?.version ?? "",
      updateCapabilities: update?.newCapabilities ?? [],
    };
  }

  listAppsJson() {
    return JSON.stringify(this.list().map((e) => this.appJson(e)).filter(Boolean));
  }

  appInfoJson(id) {
    const app = this.appJson(this.mustGet(id));
    if (!app) throw new Error(`${id} has no versions`);
    return JSON.stringify(app);
  }

  launch(id) {
    // Like the desktop poll loop: the shell opens the app after this event.
    queueMicrotask(() => this.onLaunch(id));
  }

  setGrant(id, capability, allowed) {
    const entry = this.mustGet(id);
    this.setGrantInner(id, capability, allowed, this.activeVersion(entry)?.version ?? "", false);
    this.save();
  }

  block(id) {
    if (!this.state.blocks.apps.includes(id)) this.state.blocks.apps.push(id);
    this.save();
  }

  unblock(id) {
    this.state.blocks.apps = this.state.blocks.apps.filter((a) => a !== id);
    this.save();
  }

  listGroupsJson() {
    return JSON.stringify(this.state.groups);
  }

  createGroup(name) {
    const n = String(name).trim();
    if (!n) throw new Error("a group needs a name");
    if (!this.state.groups.includes(n)) this.state.groups.push(n);
    this.save();
  }

  setGroup(id, group, member) {
    const entry = this.mustGet(id);
    if (member) {
      if (!this.state.groups.includes(group)) this.state.groups.push(group);
      if (!entry.groups.includes(group)) entry.groups.push(group);
    } else {
      entry.groups = entry.groups.filter((g) => g !== group);
    }
    this.save();
  }

  /** Removes the app from the library. Its data, grants and package files stay (as `Hub::remove`). */
  remove(id) {
    this.mustGet(id);
    delete this.state.apps[id];
    delete this.state.updates[id];
    this.save();
  }

  pin(id, version) {
    const entry = this.mustGet(id);
    if (version && !entry.versions.some((v) => v.version === version)) throw new Error(`${id} has no version ${version}`);
    entry.pinned = version || null;
    this.save();
  }

  blockPublisher(key) {
    const k = String(key).trim();
    if (!k) throw new Error("an unsigned app has no publisher key to block");
    if (!this.state.blocks.publishers.includes(k)) this.state.blocks.publishers.push(k);
    this.save();
  }

  unblockPublisher(key) {
    const k = String(key).trim();
    this.state.blocks.publishers = this.state.blocks.publishers.filter((p) => p !== k);
    this.save();
  }

  // -- `plinth:hub`: asynchronous calls ---------------------------------------

  /** `{ hits: [{ id, name, version, description, source }], errors }` as JSON (`search_sources`). */
  async search(query) {
    const hits = [];
    const errors = [];
    for (const [name, base] of this.sources()) {
      try {
        const registry = await loadRegistry(base, this.fetchImpl);
        for (const app of searchApps(registry.apps, query)) {
          hits.push({ id: app.id, name: app.name, version: app.latest, description: app.summary ?? "", source: name });
        }
      } catch (err) {
        errors.push(`${name}: ${message(err)}`);
      }
    }
    return JSON.stringify({ hits, errors });
  }

  /** The latest version of `id` in the registry at `base`, or null (`SourceClient::latest_version`). */
  async latestFrom(base, id) {
    const registry = await loadRegistry(base, this.fetchImpl);
    const summary = registry.apps.find((a) => a.id.toLowerCase() === id.toLowerCase());
    if (!summary) return null;
    const version = latestVersion(await loadAppDocument(registry, summary.id, this.fetchImpl));
    return version ? { registry, id: summary.id, version } : null;
  }

  async download(found) {
    const bytes = await fetchBytes(packageUrl(found.registry, found.version.sha256), this.fetchImpl);
    return this.addPackage(bytes, found.version, found.id, found.registry);
  }

  /** Installs the latest version of `id` from the first source that lists it (`install_from_sources`). */
  async install(id) {
    const errors = [];
    for (const [name, base] of this.sources()) {
      let found;
      try {
        found = await this.latestFrom(base, id);
      } catch (err) {
        errors.push(`${name}: ${message(err)}`);
        continue;
      }
      if (!found) continue;
      const added = await this.download(found);
      this.mustGet(added).registry = { name, base };
      this.mustGet(added).source = "registry";
      this.save();
      return added;
    }
    throw new Error(errors.length ? `no configured source could give ${id} (${errors.join("; ")})` : `no configured source lists ${id}`);
  }

  /** `{ updates: [{ id, name, current, version, source, newCapabilities, pinned }], errors }` as JSON (`check_updates`). */
  async checkUpdates(only = "") {
    const updates = [];
    const errors = [];
    if (only) this.mustGet(only);
    for (const entry of this.list()) {
      if (only && only !== entry.id) continue;
      if (!entry.registry) {
        delete this.state.updates[entry.id];
        continue;
      }
      const newest = this.newestInstalled(entry.id);
      const current = newest?.version ?? "";
      let latest;
      try {
        latest = await this.latestFrom(entry.registry.base, entry.id);
      } catch (err) {
        errors.push(`${entry.id}: ${message(err)}`);
        continue;
      }
      if (latest && isNewer(latest.version.version, current)) {
        const old = newest?.capabilities ?? [];
        const caps = (latest.version.capabilities ?? []).map((c) => c.name);
        const found = { version: latest.version.version, source: entry.registry.name, newCapabilities: caps.filter((c) => !old.includes(c)) };
        this.state.updates[entry.id] = found;
        updates.push({ id: entry.id, name: entry.name, current, version: found.version, source: found.source, newCapabilities: found.newCapabilities, pinned: entry.pinned ?? "" });
      } else {
        delete this.state.updates[entry.id];
      }
    }
    this.save();
    return JSON.stringify({ updates, errors });
  }

  /** Installs the newest version of `id` from its source (`apply_update`). Returns the new version, or "" if it is up to date. */
  async update(id) {
    const entry = this.mustGet(id);
    if (!entry.registry) throw new Error(`${id} was added from a file; it has no source to update from`);
    const current = this.newestInstalled(id)?.version ?? "";
    const latest = await this.latestFrom(entry.registry.base, id);
    if (!latest) throw new Error(`the source ${entry.registry.name} no longer lists ${id}`);
    if (!isNewer(latest.version.version, current)) {
      delete this.state.updates[id];
      this.save();
      return "";
    }
    const added = await this.download(latest);
    if (added !== id) throw new Error(`source ${entry.registry.name} gave a package for ${added}, not ${id}`);
    delete this.state.updates[id];
    this.save();
    return latest.version.version;
  }
}
