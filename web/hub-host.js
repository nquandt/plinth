// The `plinth:hub` backend of the web host (docs/web-hub.md, docs/HUB.md
// §4.1, §12.2, SPEC.md §18 browse mode), for the Hub app (`examples/hub`)
// in the browser. It is the web twin of `HubService` in `crates/plinth-hub`:
// `listApps`, `appInfo`, `search` and `checkUpdates` give JSON with the
// same fields, and the rules (risk-level defaults, consent, re-consent when
// a version adds capabilities, blocks, pins) are the same.
//
// Browse mode: the library IS the registry listing. Each app of the
// registries (`sources`) is in the library; nothing is installed. The
// browser keeps only the user's state: grants, blocks, pins, groups, the
// apps that the user removed (hidden in this browser), and for each app
// the version that the user accepted ("known"). When the registry has a
// newer version, `checkUpdates` reports it; `update` accepts it. A package
// is downloaded and checked (digest, manifest, signature) each time the app
// opens (`packageBytes`).
//
// `plinth:hub` calls are synchronous in the app. Thus `refresh()` reads
// the registries (the app list and each app document) before the Hub app
// starts and again on `checkUpdates`; the state is in memory and goes to
// `persist` (IndexedDB in the browser) after each change, in order.
//
// No DOM code: `test/run-hub-host.mjs` runs it in Node.

import { loadRegistry, loadAppDocument, searchApps, packageUrl, fetchBytes } from "./registry-client.js";
import { checkPackage } from "./hub-integrity.js";

/**
 * The capability map (`crates/plinth-link/src/capabilities.rs`): risk level
 * and description. A name that is not in it is "medium" with no
 * description, the same rule as `plinth_hub::risk_of` ("safer to ask").
 */
const CAPABILITIES = {
  "store.kv": { risk: "low", description: "save data on this device" },
  "files.private": { risk: "low", description: "save files on this device" },
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

/** None and Low are granted with no question (docs/HUB.md §7.2). */
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

function byVersionDesc(a, b) {
  return isNewer(a.version, b.version) ? -1 : isNewer(b.version, a.version) ? 1 : 0;
}

function emptyState() {
  return {
    schema: "plinth.web-hub-state/2",
    known: {},
    removed: [],
    pinned: {},
    groups: [],
    appGroups: {},
    grants: {},
    blocks: { apps: [], publishers: [] },
    updates: {},
  };
}

function message(err) {
  return String(err?.message ?? err);
}

// -- Persistence ---------------------------------------------------------------

/** A store in memory (tests, and a fallback when IndexedDB is not there). */
export function memoryPersist() {
  let state = null;
  return {
    async load() {
      return state === null ? null : JSON.parse(state);
    },
    async save(value) {
      state = JSON.stringify(value);
    },
  };
}

/** The browser store: the state in one IndexedDB database. */
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
  };
}

// -- The Hub -----------------------------------------------------------------

export class HubHost {
  /**
   * `persist`: `memoryPersist()` or `indexedDbPersist()`.
   * `sources`: `[[name, base URL]]`, the registries (the shell gives the one
   * that serves the page).
   * `onLaunch(id)`: the shell opens the app (`launch` returns at once).
   * `hide`: app ids that are not in the library (the Hub app itself).
   */
  constructor({ persist = memoryPersist(), sources = [], hide = [], onLaunch = () => {}, fetchImpl = globalThis.fetch, now = () => Math.floor(Date.now() / 1000) } = {}) {
    this.persist = persist;
    this.sourceList = sources;
    this.hide = new Set(hide);
    this.onLaunch = onLaunch;
    this.fetchImpl = (...args) => fetchImpl(...args);
    this.now = now;
    this.state = emptyState();
    /** id -> `{ id, name, publisher, source, registry, versions }` (newest first), from the registries. */
    this.catalog = new Map();
    this.sourceErrors = [];
    this.saving = Promise.resolve();
    /** Called after each change of the state (`save`), for example to tell the Hub app frame. */
    this.onChange = () => {};
  }

  /**
   * The state and the catalog as plain data, for the Hub app frame
   * (`FrameHub`): a sandboxed frame cannot read IndexedDB or ask the
   * registries itself.
   */
  snapshot() {
    return { state: structuredClone(this.state), catalog: [...this.catalog.values()], sourceErrors: [...this.sourceErrors] };
  }

  /** Takes a snapshot (each field is optional). The state must have the current schema. */
  applySnapshot({ state, catalog, sourceErrors } = {}) {
    if (state !== undefined) {
      if (!state || typeof state !== "object" || state.schema !== emptyState().schema) throw new Error("the Hub state has an unknown schema");
      this.state = { ...emptyState(), ...structuredClone(state) };
    }
    if (Array.isArray(catalog)) this.catalog = new Map(catalog.map((e) => [e.id, e]));
    if (Array.isArray(sourceErrors)) this.sourceErrors = sourceErrors.map(String);
  }

  /** Reads the stored state and the registries. Call it before the Hub app starts. */
  async load() {
    const stored = await this.persist.load();
    this.state = { ...emptyState(), ...(stored?.schema === emptyState().schema ? stored : {}) };
    await this.refresh();
    return this;
  }

  /** Writes the state; the writes run in order. */
  save() {
    const snapshot = structuredClone(this.state);
    this.saving = this.saving.then(() => this.persist.save(snapshot)).catch((err) => console.error("the Hub cannot save its state:", err));
    this.onChange();
    return this.saving;
  }

  /** Resolves when every write so far is done. */
  flush() {
    return this.saving;
  }

  sources() {
    return [...this.sourceList].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  }

  /**
   * Reads the app list and the app documents of every source. The first
   * source that lists an id gives it. An app seen for the first time gets
   * its newest version as "known" and its None/Low grants by default.
   */
  async refresh() {
    const catalog = new Map();
    const errors = [];
    for (const [name, base] of this.sources()) {
      try {
        const registry = await loadRegistry(base, this.fetchImpl);
        for (const summary of registry.apps) {
          if (catalog.has(summary.id) || this.hide.has(summary.id)) continue;
          const doc = await loadAppDocument(registry, summary.id, this.fetchImpl);
          const versions = (doc.versions ?? [])
            .filter((v) => !v.yanked)
            .map((v) => ({
              version: v.version,
              sha256: v.sha256,
              core: v.core ?? null,
              signer: v.signer ?? "",
              capabilities: (v.capabilities ?? []).map((c) => c.name),
              rationales: Object.fromEntries((v.capabilities ?? []).map((c) => [c.name, c.rationale ?? ""])),
            }))
            .sort(byVersionDesc);
          if (versions.length === 0) continue;
          catalog.set(summary.id, { id: summary.id, name: doc.name ?? summary.name, publisher: doc.publisher ?? summary.publisher ?? "", source: name, registry, versions });
        }
      } catch (err) {
        errors.push(`${name}: ${message(err)}`);
      }
    }
    this.catalog = catalog;
    this.sourceErrors = errors;
    let changed = false;
    for (const entry of catalog.values()) {
      if (this.state.known[entry.id] && entry.versions.some((v) => v.version === this.state.known[entry.id])) continue;
      if (this.state.known[entry.id]) {
        // The known version is gone (withdrawn): accept the newest one that
        // is not newer, else the oldest.
        const older = entry.versions.find((v) => !isNewer(v.version, this.state.known[entry.id]));
        this.state.known[entry.id] = (older ?? entry.versions.at(-1)).version;
      } else {
        this.state.known[entry.id] = entry.versions[0].version;
      }
      this.grantLowRisk(entry.id, this.knownVersion(entry));
      changed = true;
    }
    if (changed) this.save();
    return errors;
  }

  grantLowRisk(id, version) {
    const grants = this.grants(id);
    for (const cap of version.capabilities) if (!grants[cap] && isLowRisk(cap)) this.setGrantInner(id, cap, true, version.version, true);
  }

  // -- The library -----------------------------------------------------------

  /** The library: every registry app that the user did not remove, sorted by id (as `Hub::list`). */
  list() {
    return [...this.catalog.values()].filter((e) => !this.state.removed.includes(e.id)).sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  }

  get(id) {
    const entry = this.catalog.get(id);
    return entry && !this.state.removed.includes(id) ? entry : null;
  }

  mustGet(id) {
    const entry = this.get(id);
    if (!entry) throw new Error(`${id} is not in the library`);
    return entry;
  }

  knownVersion(entry) {
    return entry.versions.find((v) => v.version === this.state.known[entry.id]) ?? entry.versions[0];
  }

  /** The versions that count as installed: the known one and the older ones (newest first). */
  installedVersions(entry) {
    const known = this.knownVersion(entry).version;
    return entry.versions.filter((v) => !isNewer(v.version, known));
  }

  /** The version that `appInfo` shows and a launch opens first: the pinned one, else the known one. */
  activeVersion(entry) {
    const pinned = this.state.pinned[entry.id];
    return (pinned && entry.versions.find((v) => v.version === pinned)) || this.knownVersion(entry);
  }

  grants(id) {
    return this.state.grants[id] ?? {};
  }

  groupsOf(id) {
    return this.state.appGroups[id] ?? [];
  }

  isAppBlocked(id) {
    return this.state.blocks.apps.includes(id);
  }

  isPublisherBlocked(key) {
    return this.state.blocks.publishers.includes(key);
  }

  isPublisherOfBlocked(id) {
    const entry = this.catalog.get(id);
    if (!entry || this.state.blocks.publishers.length === 0) return false;
    return this.installedVersions(entry).some((v) => v.signer && this.isPublisherBlocked(v.signer));
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

  /** The version to run: the pinned one, else the newest installed one whose capabilities are all decided, else the known one (`Hub::runnable_version`). */
  runnableVersion(id) {
    const entry = this.mustGet(id);
    const pinned = this.state.pinned[id];
    const pin = pinned && entry.versions.find((v) => v.version === pinned);
    if (pin) return pin;
    return this.installedVersions(entry).find((v) => this.needsConsent(id, v.capabilities).length === 0) ?? this.knownVersion(entry);
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

  /**
   * Downloads `version` of `id` and checks it against the registry (digest,
   * manifest, signature; `checkPackage`). Returns the package bytes.
   */
  async packageBytes(id, version) {
    const entry = this.mustGet(id);
    const bytes = await fetchBytes(packageUrl(entry.registry, version.sha256), this.fetchImpl);
    const { signature } = await checkPackage(bytes, { ...version, signer: version.signer || undefined }, id);
    if (signature.key && this.isPublisherBlocked(signature.key)) throw new Error(`publisher ${signature.publisher} (${signature.key}) is blocked`);
    return bytes;
  }

  // -- `plinth:hub`: synchronous calls ----------------------------------------
  // Each one throws an Error on failure; `plinth-web.js` answers "denied".

  /** One library app as the JSON object of `listApps`/`appInfo` (`HubService::app_json`). */
  appJson(entry) {
    const active = this.activeVersion(entry);
    const grants = this.grants(entry.id);
    const update = this.state.updates[entry.id] ?? null;
    return {
      id: entry.id,
      name: entry.name,
      version: active.version,
      publisher: entry.publisher,
      signer: active.signer,
      source: entry.source,
      blocked: this.isAppBlocked(entry.id),
      publisherBlocked: this.isPublisherOfBlocked(entry.id),
      groups: this.groupsOf(entry.id),
      capabilities: active.capabilities.map((name) => ({
        name,
        risk: riskOf(name),
        description: descriptionOf(name),
        rationale: active.rationales?.[name] ?? "",
        decided: Boolean(grants[name]),
        allowed: grants[name]?.decision === "allowed",
        byDefault: Boolean(grants[name]?.byDefault),
      })),
      pinned: this.state.pinned[entry.id] ?? "",
      versions: this.installedVersions(entry).map((v) => ({ version: v.version, signer: v.signer, capabilities: v.capabilities })),
      update: update?.version ?? "",
      updateCapabilities: update?.newCapabilities ?? [],
    };
  }

  listAppsJson() {
    return JSON.stringify(this.list().map((e) => this.appJson(e)));
  }

  appInfoJson(id) {
    return JSON.stringify(this.appJson(this.mustGet(id)));
  }

  launch(id) {
    // Like the desktop poll loop: the shell opens the app after this event.
    queueMicrotask(() => this.onLaunch(id));
  }

  setGrant(id, capability, allowed) {
    const entry = this.mustGet(id);
    this.setGrantInner(id, capability, allowed, this.activeVersion(entry).version, false);
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
    this.mustGet(id);
    const groups = this.groupsOf(id).filter((g) => g !== group);
    if (member) {
      if (!this.state.groups.includes(group)) this.state.groups.push(group);
      groups.push(group);
    }
    this.state.appGroups[id] = groups;
    this.save();
  }

  /** Hides the app in this browser. Its data and grants stay (as `Hub::remove`); `install` shows it again. */
  remove(id) {
    this.mustGet(id);
    this.state.removed.push(id);
    delete this.state.updates[id];
    this.save();
  }

  pin(id, version) {
    const entry = this.mustGet(id);
    if (version && !this.installedVersions(entry).some((v) => v.version === version)) throw new Error(`${id} has no version ${version}`);
    if (version) this.state.pinned[id] = version;
    else delete this.state.pinned[id];
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

  /**
   * In browse mode every registry app is in the library already: `install`
   * shows an app that the user removed, and reads the registries again for
   * an app that is new there. Returns the app id.
   */
  async install(id) {
    if (!this.catalog.has(id)) await this.refresh();
    if (!this.catalog.has(id)) {
      throw new Error(this.sourceErrors.length ? `no configured source could give ${id} (${this.sourceErrors.join("; ")})` : `no configured source lists ${id}`);
    }
    if (this.isBlocked(id)) throw new Error(`${id} is blocked`);
    this.state.removed = this.state.removed.filter((r) => r !== id);
    this.save();
    return id;
  }

  /** `{ updates: [{ id, name, current, version, source, newCapabilities, pinned }], errors }` as JSON (`check_updates`). */
  async checkUpdates(only = "") {
    if (only) this.mustGet(only);
    const errors = await this.refresh();
    const updates = [];
    for (const entry of this.list()) {
      if (only && only !== entry.id) continue;
      const known = this.knownVersion(entry);
      const newest = entry.versions[0];
      if (isNewer(newest.version, known.version)) {
        const found = { version: newest.version, source: entry.source, newCapabilities: newest.capabilities.filter((c) => !known.capabilities.includes(c)) };
        this.state.updates[entry.id] = found;
        updates.push({ id: entry.id, name: entry.name, current: known.version, version: found.version, source: found.source, newCapabilities: found.newCapabilities, pinned: this.state.pinned[entry.id] ?? "" });
      } else {
        delete this.state.updates[entry.id];
      }
    }
    this.save();
    return JSON.stringify({ updates, errors });
  }

  /**
   * Accepts the newest registry version of `id` (`apply_update`). Returns
   * the new version, or "" if it is up to date. Its new Medium/High
   * capabilities are asked for when it opens; until then the previous
   * version runs (`runnableVersion`).
   */
  async update(id) {
    this.mustGet(id);
    await this.refresh();
    const entry = this.get(id);
    if (!entry) throw new Error(`the source no longer lists ${id}`);
    const known = this.knownVersion(entry);
    const newest = entry.versions[0];
    delete this.state.updates[id];
    if (!isNewer(newest.version, known.version)) {
      this.save();
      return "";
    }
    if (known.signer && newest.signer && known.signer !== newest.signer) {
      throw new Error(`${id} ${newest.version} is signed by a different key (${newest.signer}) than ${known.version} (${known.signer}); key rotation is not supported yet`);
    }
    this.state.known[id] = newest.version;
    this.grantLowRisk(id, newest);
    this.save();
    return newest.version;
  }
}

/** The calls of `plinth:hub` that read the registries: the Hub app frame asks the page for them. */
export const REMOTE_HUB_CALLS = ["search", "install", "checkUpdates", "update"];

/**
 * The `plinth:hub` backend inside the sandboxed frame of the Hub app
 * (`app-frame.js`). The frame has an opaque origin, so it cannot read
 * IndexedDB or ask the registries (no CORS for the origin "null"), and the
 * synchronous calls of the app cannot wait for the page. Thus the frame
 * keeps a copy of the page's `HubHost`: the page sends a `snapshot()` at
 * the start; the synchronous calls run on the copy and send each new state
 * to the page (`hub-save`); `launch` goes to the page (`hub-launch`); the
 * calls that read the registries run in the page (`hub-call`), which
 * answers with the result and a new snapshot. When the page changes the
 * state (the consent window), it sends `hub-state`.
 */
export class FrameHub extends HubHost {
  /** `send(msg)` posts to the page; `ask(type, fields)` posts and resolves with the answer. */
  constructor(snapshot, { send, ask }) {
    super({
      persist: memoryPersist(),
      onLaunch: (id) => send({ type: "hub-launch", id }),
      fetchImpl: () => Promise.reject(new Error("the Hub app frame does not read the registries")),
    });
    this.send = send;
    this.ask = ask;
    this.applySnapshot(snapshot);
  }

  /** Sends the state at once, so that the page has it before a later `launch` or `hub-call`. */
  save() {
    this.send({ type: "hub-save", state: structuredClone(this.state) });
    return this.saving;
  }

  async remote(method, args) {
    const answer = await this.ask("hub-call", { method, args });
    if (answer?.snapshot) this.applySnapshot(answer.snapshot);
    if (!answer?.ok) throw new Error(answer?.error ?? "the Hub page did not answer");
    return answer.value;
  }

  search(query) {
    return this.remote("search", [String(query)]);
  }

  install(id) {
    return this.remote("install", [String(id)]);
  }

  checkUpdates(only = "") {
    return this.remote("checkUpdates", [String(only)]);
  }

  update(id) {
    return this.remote("update", [String(id)]);
  }

  refresh() {
    return Promise.resolve(this.sourceErrors);
  }
}
