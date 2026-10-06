// Storage of the web App Hub (docs/web-hub.md §4): the hub page (the
// parent) owns all storage. Each app runs in a sandboxed frame with an
// opaque origin (`app-frame.html`), so it cannot read the storage of the
// hub or of a different app. The frame asks for the kv operations of the
// app through `postMessage`, and the parent keeps them under a namespace
// for each app.
//
// `plinth:store` calls are synchronous in the app, and `postMessage` is not.
// Thus the parent sends a snapshot of the app's kv data (and the quota) at
// the start; the frame answers the app from its local copy and sends each
// write to the parent (the same idea as the clipboard cache in
// `plinth-web.js`). The frame and the parent apply the same quota, so the
// parent gets no write that it must refuse.
//
// No DOM code: `test/run-hub-storage.mjs` runs this file directly. The
// backing store is anything with the `Storage` interface (`localStorage`
// in the browser, a small Map-backed object in the test).

/** The kv quota of one app, in characters (UTF-16 code units) of keys and values. */
export const KV_QUOTA = 512 * 1024;

/** The longest key that the frame accepts (characters). */
export const MAX_KEY = 1024;

/** The tag on each message between the hub page and the app frame. */
export const CHANNEL = "plinth-frame/1";

const PREFIX = "plinth-hub:";

function norm(id) {
  return String(id).toLowerCase();
}

function kvPrefix(id) {
  return `${PREFIX}kv:${norm(id)}:`;
}

function grantsKey(id) {
  return `${PREFIX}grants:${norm(id)}`;
}

/** The size that counts against the quota for one entry. */
export function entrySize(key, value) {
  return key.length + value.length;
}

/** A `Storage`-shaped object on a `Map` (for tests and as a fallback). */
export function memoryStorage(map = new Map()) {
  return {
    get length() {
      return map.size;
    },
    key(i) {
      return [...map.keys()][i] ?? null;
    },
    getItem(k) {
      return map.has(k) ? map.get(k) : null;
    },
    setItem(k, v) {
      map.set(k, String(v));
    },
    removeItem(k) {
      map.delete(k);
    },
  };
}

function storageKeys(storage) {
  const out = [];
  for (let i = 0; i < storage.length; i++) {
    const k = storage.key(i);
    if (k !== null) out.push(k);
  }
  return out;
}

/**
 * The hub page's storage: kv data and grant records for each app.
 * `storage` keeps data across sessions (`localStorage`); `session` keeps
 * the decisions that last for one session (`sessionStorage`).
 */
export class HubStore {
  constructor(storage, session = memoryStorage()) {
    this.storage = storage;
    this.session = session;
  }

  /** The kv data of `id` as a plain object. */
  kvSnapshot(id) {
    const prefix = kvPrefix(id);
    const out = {};
    for (const k of storageKeys(this.storage)) {
      if (k.startsWith(prefix)) out[k.slice(prefix.length)] = this.storage.getItem(k);
    }
    return out;
  }

  /** The characters that the kv data of `id` uses. */
  kvUsage(id) {
    return Object.entries(this.kvSnapshot(id)).reduce((n, [k, v]) => n + entrySize(k, v), 0);
  }

  /** Sets one key. False (and no change) if the app goes over `quota` or the browser storage is full. */
  kvSet(id, key, value, quota = KV_QUOTA) {
    const prefix = kvPrefix(id);
    const old = this.storage.getItem(prefix + key);
    const used = this.kvUsage(id) - (old === null ? 0 : entrySize(key, old)) + entrySize(key, value);
    if (used > quota) return false;
    try {
      this.storage.setItem(prefix + key, value);
    } catch {
      return false; // QuotaExceededError of the browser
    }
    return true;
  }

  kvDelete(id, key) {
    this.storage.removeItem(kvPrefix(id) + key);
  }

  /** Removes all kv data of `id`. */
  kvClear(id) {
    const prefix = kvPrefix(id);
    for (const k of storageKeys(this.storage)) if (k.startsWith(prefix)) this.storage.removeItem(k);
  }

  /** The stored grant record of `id`, or null. */
  grants(id) {
    return parseJson(this.storage.getItem(grantsKey(id)));
  }

  /** The decisions of `id` that last for this session, or null. */
  sessionGrants(id) {
    return parseJson(this.session.getItem(grantsKey(id)));
  }

  saveGrants(id, record, sessionRecord) {
    if (record) this.storage.setItem(grantsKey(id), JSON.stringify(record));
    if (sessionRecord) this.session.setItem(grantsKey(id), JSON.stringify(sessionRecord));
  }

  /** Removes the grant records of `id`: the next run asks again. */
  forgetGrants(id) {
    this.storage.removeItem(grantsKey(id));
    this.session.removeItem(grantsKey(id));
  }

  /**
   * Every app that has a grant record or kv data:
   * `[{ id, name, record, usage }]`, sorted by name.
   */
  knownApps() {
    const ids = new Map();
    for (const k of storageKeys(this.storage)) {
      if (k.startsWith(`${PREFIX}grants:`)) {
        const record = parseJson(this.storage.getItem(k));
        const id = k.slice(`${PREFIX}grants:`.length);
        ids.set(id, record?.id ?? id);
      } else if (k.startsWith(`${PREFIX}kv:`)) {
        const rest = k.slice(`${PREFIX}kv:`.length);
        const id = rest.slice(0, rest.indexOf(":"));
        if (!ids.has(id)) ids.set(id, id);
      }
    }
    return [...ids.values()]
      .map((id) => {
        const record = this.grants(id);
        return { id, name: record?.name ?? id, record, usage: this.kvUsage(id) };
      })
      .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  }
}

function parseJson(text) {
  if (text === null || text === undefined) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// -- The frame side ----------------------------------------------------------

/**
 * The kv store of an app in its frame (the `kvStore` that `plinth-web.js`
 * expects): answers from `snapshot` at once, and calls `send(message)` for
 * each write. `set` returns false over `quota` (the app gets
 * `denied(refused)`).
 */
export function frameKvStore(snapshot, quota, send) {
  const map = new Map(Object.entries(snapshot ?? {}).filter(([k, v]) => typeof k === "string" && typeof v === "string"));
  let used = 0;
  for (const [k, v] of map) used += entrySize(k, v);
  return {
    get(key) {
      return map.has(key) ? map.get(key) : null;
    },
    set(key, value) {
      if (key.length > MAX_KEY) return false;
      const old = map.get(key);
      const next = used - (old === undefined ? 0 : entrySize(key, old)) + entrySize(key, value);
      if (next > quota) return false;
      map.set(key, value);
      used = next;
      send({ channel: CHANNEL, type: "kv-set", key, value });
      return true;
    },
    delete(key) {
      const old = map.get(key);
      if (old === undefined) return;
      map.delete(key);
      used -= entrySize(key, old);
      send({ channel: CHANNEL, type: "kv-delete", key });
    },
    keys() {
      return [...map.keys()];
    },
    get used() {
      return used;
    },
  };
}

// -- Messages ----------------------------------------------------------------

const FRAME_TYPES = {
  ready: {},
  started: {},
  failed: { message: "string" },
  "kv-set": { key: "string", value: "string" },
  "kv-delete": { key: "string" },
  "clipboard-write": { text: "string" },
  "clipboard-read": { id: "number" },
  dialog: { id: "number", kind: "string", message: "string" },
  "net-fetch": { id: "number", url: "string", method: "string" },
  size: { height: "number" },
};

/**
 * Checks a message from an app frame. The frame is not trusted: it runs
 * the code of an app (through the web host). Returns the message, or null
 * if it is not a valid frame message.
 */
export function checkFrameMessage(data) {
  if (!data || typeof data !== "object" || data.channel !== CHANNEL) return null;
  const shape = Object.hasOwn(FRAME_TYPES, data.type) ? FRAME_TYPES[data.type] : null;
  if (!shape) return null;
  for (const [field, type] of Object.entries(shape)) {
    if (typeof data[field] !== type) return null;
  }
  if (data.type === "dialog" && !["alert", "confirm", "prompt"].includes(data.kind)) return null;
  if ((data.type === "kv-set" || data.type === "kv-delete") && data.key.length > MAX_KEY) return null;
  if (data.type === "net-fetch") {
    const headersOk = Array.isArray(data.headers) && data.headers.every((h) => Array.isArray(h) && h.length === 2 && h.every((x) => typeof x === "string"));
    if (!headersOk || (data.body !== null && typeof data.body !== "string")) return null;
  }
  if (data.type === "size" && !(Number.isFinite(data.height) && data.height >= 0)) return null;
  return data;
}

/**
 * Applies a kv message from the frame of app `id` to the store. Returns
 * true if the message was a kv message (handled here), false for the
 * other types (the page handles them). The app id comes from the frame
 * that the page made, never from the message.
 */
export function applyKvMessage(store, id, msg, quota = KV_QUOTA) {
  if (msg.type === "kv-set") {
    store.kvSet(id, msg.key, msg.value, quota);
    return true;
  }
  if (msg.type === "kv-delete") {
    store.kvDelete(id, msg.key);
    return true;
  }
  return false;
}

/**
 * The `start` message to an app frame: the package and the core (as
 * `ArrayBuffer`s, transferred), the kv snapshot of this app only, the quota,
 * and the capabilities that the user refused.
 */
export function startMessage(store, id, { pkg, core, refused, quota = KV_QUOTA }) {
  return { channel: CHANNEL, type: "start", pkg, core, kv: store.kvSnapshot(id), quota, refused: [...refused] };
}
