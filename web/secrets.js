// `plinth:secrets` on the web (core 1.13, docs/SECRETS.md): the secrets of an
// app. A browser has no keychain. The page makes one AES-GCM 256 key for the
// browser profile (WebCrypto, `extractable: false`) and keeps it as a
// `CryptoKey` in IndexedDB. Only encrypted values are stored: a random
// 12-byte IV for each write, and the app id, the instance and the name as
// additional authenticated data, so a record cannot move to another name or
// app. This protects against a copy of the profile on disk, not against code
// that runs in the page. The sandboxed frame has an opaque origin and cannot
// use IndexedDB: the page keeps the secrets for the frame (`frame-host.js`)
// and checks the capability again.
//
// A `SecretSpace` runs the calls of one app, one at a time in call order, on
// a backend. `call(op, name, value)` (op "set", "get", "remove" or "names")
// answers `[ok, n, text, error]` and never throws (docs/SECRETS.md §5).
// A browser with no IndexedDB or no `crypto.subtle` answers
// "denied:unsupported" and never keeps a value in a weaker place.
//
// No DOM code: `test/run-secrets.mjs` runs this file in a browser.

/** The capability of the secrets. */
export const SECRETS_CAPABILITY = "secrets";
/** The most characters in a name, the most bytes (UTF-8) in a value, the most secrets of one app. */
export const MAX_NAME = 64;
export const MAX_VALUE = 16 * 1024;
export const MAX_SECRETS = 64;
/** The instance of an app (docs/STORAGE.md "Instances"). */
export const INSTANCE = "default";

const DB_NAME = "plinth-secrets";
const encoder = new TextEncoder();
const decoder = new TextDecoder();

const ok = (n = 0, text = "") => [true, n, text, null];
const fail = (error) => [false, 0, "", error];

/** True if `name` is a name of a secret: 1 to 64 characters from `a-z A-Z 0-9 . _ -`. */
export function validName(name) {
  return typeof name === "string" && /^[A-Za-z0-9._-]{1,64}$/.test(name);
}

/** True if the call breaks a limit of §2 (a bad name, or a value over 16 KiB). */
export function overLimit(op, name, value) {
  if (op !== "names" && !validName(name)) return true;
  return op === "set" && (typeof value !== "string" || encoder.encode(value).length > MAX_VALUE);
}

/** True if this page can keep secrets: it has IndexedDB and WebCrypto `subtle`. */
export function secretsSupported(idb = globalThis.indexedDB, subtle = globalThis.crypto?.subtle) {
  return Boolean(idb) && Boolean(subtle) && typeof subtle.generateKey === "function";
}

/** A backend in memory, for tests of `SecretSpace` (the values are not encrypted). */
export function memorySecretBackend(map = new Map()) {
  return {
    supported: true,
    async get(name) {
      return map.has(name) ? map.get(name) : undefined;
    },
    async put(name, value) {
      map.set(name, value);
    },
    async delete(name) {
      map.delete(name);
    },
    async names() {
      return [...map.keys()];
    },
    async clear() {
      map.clear();
    },
  };
}

/** Opens (and makes, once) the database: the object stores `keys` and `values`. */
function openDb(idb) {
  return new Promise((resolve, reject) => {
    const req = idb.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      req.result.createObjectStore("keys");
      req.result.createObjectStore("values");
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

/**
 * A backend on the page's IndexedDB and WebCrypto. The database
 * `plinth-secrets` has the object store `keys` (the key "profile": the
 * `CryptoKey`) and the object store `values` (keys `[space, instance,
 * name]`, values `{ iv, data }`, both `ArrayBuffer`). `space` is the app id
 * in lower case (the same namespace rule as `idbFileBackend`).
 */
export function idbSecretBackend(space, idb = globalThis.indexedDB, subtle = globalThis.crypto?.subtle) {
  const ns = String(space).toLowerCase();
  const supported = secretsSupported(idb, subtle);
  let dbPromise = null;
  let keyPromise = null;
  const db = () => (dbPromise ??= openDb(idb));
  function run(store, mode, fn) {
    return db().then(
      (d) =>
        new Promise((resolve, reject) => {
          const tx = d.transaction(store, mode);
          let value;
          fn(tx.objectStore(store), (v) => (value = v));
          tx.oncomplete = () => resolve(value);
          tx.onerror = () => reject(tx.error);
          tx.onabort = () => reject(tx.error);
        }),
    );
  }
  /** The key of the profile. Made one time; two tabs that race both end with the key that was stored first. */
  function profileKey() {
    keyPromise ??= (async () => {
      const found = await run("keys", "readonly", (s, set) => {
        const r = s.get("profile");
        r.onsuccess = () => set(r.result);
      });
      if (found) return found;
      const fresh = await subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
      return run("keys", "readwrite", (s, set) => {
        const r = s.get("profile");
        r.onsuccess = () => {
          if (r.result) set(r.result);
          else {
            s.put(fresh, "profile");
            set(fresh);
          }
        };
      });
    })();
    // A failure is not kept: the next call tries again.
    keyPromise.catch(() => (keyPromise = null));
    return keyPromise;
  }
  const aad = (name) => encoder.encode(`${ns}\u0000${INSTANCE}\u0000${name}`);
  return {
    supported,
    async get(name) {
      const rec = await run("values", "readonly", (s, set) => {
        const r = s.get([ns, INSTANCE, name]);
        r.onsuccess = () => set(r.result);
      });
      if (!rec) return undefined;
      const key = await profileKey();
      const plain = await subtle.decrypt({ name: "AES-GCM", iv: rec.iv, additionalData: aad(name) }, key, rec.data);
      return decoder.decode(plain);
    },
    async put(name, value) {
      const key = await profileKey();
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const data = await subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad(name) }, key, encoder.encode(value));
      await run("values", "readwrite", (s) => s.put({ iv: iv.buffer, data }, [ns, INSTANCE, name]));
    },
    async delete(name) {
      await run("values", "readwrite", (s) => s.delete([ns, INSTANCE, name]));
    },
    async names() {
      return run("values", "readonly", (s, set) => {
        const out = [];
        const r = s.openCursor(IDBKeyRange.bound([ns, INSTANCE, ""], [ns, INSTANCE, "￿"]));
        r.onsuccess = () => {
          const c = r.result;
          if (!c) return set(out);
          out.push(c.key[2]);
          c.continue();
        };
      });
    },
    /** Deletes every secret of the app (uninstall, docs/SECRETS.md §3 rule 3). */
    async clear() {
      await run("values", "readwrite", (s) => s.delete(IDBKeyRange.bound([ns], [ns, "￿"])));
    },
  };
}

/**
 * Deletes every secret of the app `appId` from the page's IndexedDB. Does
 * nothing if there is none, or if the browser has no IndexedDB. Resolves
 * with nothing; rejects if the database fails.
 */
export async function clearAppSecrets(appId, idb = globalThis.indexedDB) {
  if (!idb) return;
  await idbSecretBackend(appId, idb, { generateKey() {} }).clear();
}

/**
 * The secrets of one app on a backend. `call(op, name, value)` answers `[ok,
 * n, text, error]` and never throws. The error of a failed store is "failed"
 * (the text never holds a value).
 */
export class SecretSpace {
  constructor(backend) {
    this.backend = backend;
    this.queue = Promise.resolve();
  }

  call(op, name, value = null) {
    const run = this.queue.then(() => this.run(op, name, value));
    this.queue = run.catch(() => {});
    return run.catch(() => fail("failed"));
  }

  async run(op, name, value) {
    const b = this.backend;
    if (overLimit(op, name, value)) return fail("denied:limit");
    if (!b.supported) return fail("denied:unsupported");
    switch (op) {
      case "set": {
        const names = await b.names();
        if (names.length >= MAX_SECRETS && !names.includes(name)) return fail("denied:limit");
        await b.put(name, value);
        return ok();
      }
      case "get": {
        const v = await b.get(name);
        return v === undefined ? ok(0, "") : ok(1, v);
      }
      case "remove":
        await b.delete(name);
        return ok();
      case "names": {
        const names = [...(await b.names())].sort();
        return ok(names.length, JSON.stringify(names));
      }
      default:
        return fail("failed");
    }
  }
}

/** The secrets of `appId` in this page, or null if the browser cannot keep them safely. */
export function pageSecrets(appId) {
  return appId && secretsSupported() ? new SecretSpace(idbSecretBackend(appId)) : null;
}
