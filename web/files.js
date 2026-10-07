// `plinth:files` on the web (core 1.11, docs/STORAGE.md §2, §3): the private
// space of an app. The same rules and results as the desktop runner
// (`crates/plinth-runner-wasmtime/src/files.rs`): the host checks every path
// (`checkPath`), a file is UTF-8 text of at most `MAX_FILE` bytes, a space
// holds at most `FILES_QUOTA` bytes, and directories exist only as parents
// of files. Each call answers the 4-element list `[ok, 0, text, error]`
// that `net.fetch` also uses.
//
// A `FileSpace` runs the calls of one app, one at a time in call order, on
// a backend: `memoryFileBackend` (Node tests and the stand-alone page) or
// `idbFileBackend` (the page that hosts sandboxed app frames keeps the files
// in its IndexedDB, one namespace per app id). The sandboxed frame never
// holds the files: it asks the page through the `files-*` bridge messages
// (`hub-storage.js`, web/README.md).
//
// No DOM code: `test/run-files.mjs` runs this file in Node.

/** The capability of the private space. */
export const FILES_CAPABILITY = "files.private";
/** The bytes of file content that one app may keep (as on the desktop). */
export const FILES_QUOTA = 50 * 1024 * 1024;
/** The largest file, in bytes. */
export const MAX_FILE = 8 * 1024 * 1024;
/** The longest path in UTF-8 bytes, the longest segment, the most segments. */
export const MAX_PATH = 1024;
export const MAX_SEGMENT = 255;
export const MAX_DEPTH = 32;

const encoder = new TextEncoder();
const utf8Length = (s) => encoder.encode(s).length;

/**
 * The reason that `segment` is refused, or null (docs/host-apis.md
 * "plinth:files"; the same rules as `check_segment` in files.rs).
 */
function segmentError(seg) {
  if (seg === "") return "empty segment";
  if (seg === "." || seg === "..") return "dot segment";
  if (utf8Length(seg) > MAX_SEGMENT) return "segment too long";
  // Control characters: C0, DEL and C1 (Rust's `char::is_control`).
  if (/[\u0000-\u001f\u007f-\u009f]/.test(seg)) return "control character";
  if (/[:*?"<>|]/.test(seg)) return "reserved character";
  if (seg.endsWith(".") || seg.endsWith(" ")) return "trailing dot or space";
  const stem = seg.split(".")[0].trimEnd().replace(/[a-z]/g, (c) => c.toUpperCase());
  if (["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].includes(stem) || /^(COM|LPT)[0-9]$/.test(stem)) return "device name";
  return null;
}

/**
 * Checks a path from an app (docs/STORAGE.md §3 rule 1). Returns
 * `{ segments }` or `{ error }` (the text after "invalid-path: "). `""` is
 * the root, allowed only when `allowRoot`.
 */
export function checkPath(path, allowRoot = false) {
  if (typeof path !== "string") return { error: "empty" };
  if (path === "") return allowRoot ? { segments: [] } : { error: "empty" };
  if (utf8Length(path) > MAX_PATH) return { error: "too long" };
  if (path.startsWith("/")) return { error: "absolute" };
  if (path.includes("\\")) return { error: "backslash" };
  const segments = path.split("/");
  if (segments.length > MAX_DEPTH) return { error: "too deep" };
  for (const seg of segments) {
    const error = segmentError(seg);
    if (error) return { error };
  }
  return { segments };
}

/** True if `name` is a name the app could have made (for listing). */
export function isValidSegment(name) {
  return segmentError(name) === null;
}

const ok = (text = "") => [true, 0, text, null];
const fail = (error) => [false, 0, "", error];

/** Compares by Unicode code point, as Rust's `str` order on the desktop. */
function byCodePoint(a, b) {
  const x = [...a];
  const y = [...b];
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    const d = x[i].codePointAt(0) - y[i].codePointAt(0);
    if (d !== 0) return d;
  }
  return x.length - y.length;
}

/**
 * A backend in memory: `Map` of path to text. Two `FileSpace`s on the same
 * map share files (as two tabs of one browser profile would).
 */
export function memoryFileBackend(map = new Map()) {
  return {
    async get(path) {
      return map.has(path) ? map.get(path) : undefined;
    },
    async put(path, text) {
      map.set(path, text);
    },
    async delete(paths) {
      for (const p of paths) map.delete(p);
    },
    /** Every file whose path starts with `prefix`: `[{ path, size }]`. */
    async scan(prefix) {
      const out = [];
      for (const [path, text] of map) if (path.startsWith(prefix)) out.push({ path, size: utf8Length(text) });
      return out;
    },
  };
}

/** A per-space view of one `Map` (keys `<space>\0<path>`), for tests with several apps. */
export function sharedMemoryBackend(shared, space) {
  const pre = `${space}\u0000`;
  return {
    async get(path) {
      return shared.has(pre + path) ? shared.get(pre + path) : undefined;
    },
    async put(path, text) {
      shared.set(pre + path, text);
    },
    async delete(paths) {
      for (const p of paths) shared.delete(pre + p);
    },
    async scan(prefix) {
      const out = [];
      for (const [key, text] of shared) if (key.startsWith(pre + prefix)) out.push({ path: key.slice(pre.length), size: utf8Length(text) });
      return out;
    },
  };
}

/**
 * A backend on the page's IndexedDB: the object store `files` of the
 * database `plinth-files`, with keys `[space, path]` and values `{ text,
 * size }`. `space` is the app id in lower case (the same namespace rule as
 * the kv data of `hub-storage.js`).
 */
export function idbFileBackend(space, idb = globalThis.indexedDB) {
  const ns = String(space).toLowerCase();
  let dbPromise = null;
  function db() {
    dbPromise ??= new Promise((resolve, reject) => {
      const req = idb.open("plinth-files", 1);
      req.onupgradeneeded = () => req.result.createObjectStore("files");
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    return dbPromise;
  }
  function run(mode, fn) {
    return db().then(
      (d) =>
        new Promise((resolve, reject) => {
          const tx = d.transaction("files", mode);
          const store = tx.objectStore("files");
          let value;
          fn(store, (v) => (value = v));
          tx.oncomplete = () => resolve(value);
          tx.onerror = () => reject(tx.error);
          tx.onabort = () => reject(tx.error);
        }),
    );
  }
  return {
    get(path) {
      return run("readonly", (s, set) => {
        const r = s.get([ns, path]);
        r.onsuccess = () => set(r.result === undefined ? undefined : r.result.text);
      });
    },
    put(path, text) {
      return run("readwrite", (s) => s.put({ text, size: utf8Length(text) }, [ns, path]));
    },
    delete(paths) {
      return run("readwrite", (s) => {
        for (const p of paths) s.delete([ns, p]);
      });
    },
    scan(prefix) {
      return run("readonly", (s, set) => {
        const out = [];
        const range = IDBKeyRange.bound([ns, prefix], [ns, `${prefix}￿`]);
        const r = s.openCursor(range);
        r.onsuccess = () => {
          const c = r.result;
          if (!c) return set(out);
          out.push({ path: c.key[1], size: c.value.size });
          c.continue();
        };
      });
    },
  };
}

/**
 * The private space of one app on a backend. `call(op, path, text)` (op
 * "read", "write", "list", "stat" or "remove") answers `[ok, 0, text,
 * error]` and never throws. Calls run one at a time, in call order.
 */
export class FileSpace {
  constructor(backend, quota = FILES_QUOTA) {
    this.backend = backend;
    this.quota = quota;
    this.queue = Promise.resolve();
  }

  call(op, path, text = null) {
    const run = this.queue.then(() => this.run(op, path, text));
    this.queue = run.catch(() => {});
    return run.catch((e) => fail(`io: ${e?.message ?? e}`));
  }

  async run(op, path, text) {
    const b = this.backend;
    const checked = checkPath(path, op === "list");
    if (checked.error) return fail(`invalid-path: ${checked.error}`);
    const segs = checked.segments;
    const full = segs.join("/");
    const under = async (p) => b.scan(p === "" ? "" : `${p}/`);
    switch (op) {
      case "read": {
        const t = await b.get(full);
        if (t !== undefined) return utf8Length(t) > MAX_FILE ? fail("too-large") : ok(t);
        return (await under(full)).length ? fail("not-a-file") : fail("not-found");
      }
      case "write": {
        if (typeof text !== "string") return fail("invalid-path: empty");
        const size = utf8Length(text);
        if (size > MAX_FILE) return fail("too-large");
        for (let i = 1; i < segs.length; i++) {
          if ((await b.get(segs.slice(0, i).join("/"))) !== undefined) return fail("not-a-directory");
        }
        if ((await under(full)).length) return fail("not-a-file");
        const old = await b.get(full);
        const used = (await b.scan("")).reduce((n, f) => n + f.size, 0);
        if (used - (old === undefined ? 0 : utf8Length(old)) + size > this.quota) return fail("quota");
        await b.put(full, text);
        return ok();
      }
      case "list": {
        if (full !== "" && (await b.get(full)) !== undefined) return fail("not-a-directory");
        const files = await under(full);
        if (full !== "" && files.length === 0) return fail("not-found");
        const skip = full === "" ? 0 : full.length + 1;
        const entries = new Map();
        for (const f of files) {
          const rest = f.path.slice(skip);
          const slash = rest.indexOf("/");
          if (slash < 0) entries.set(rest, { kind: "file", name: rest, size: f.size });
          else entries.set(rest.slice(0, slash), { kind: "dir", name: rest.slice(0, slash), size: 0 });
        }
        const list = [...entries.values()].sort((x, y) => byCodePoint(x.name, y.name));
        return ok(JSON.stringify(list));
      }
      case "stat": {
        const name = segs[segs.length - 1];
        const t = await b.get(full);
        if (t !== undefined) return ok(JSON.stringify({ kind: "file", name, size: utf8Length(t) }));
        if ((await under(full)).length) return ok(JSON.stringify({ kind: "dir", name, size: 0 }));
        return ok("null");
      }
      case "remove": {
        const paths = (await under(full)).map((f) => f.path);
        if ((await b.get(full)) !== undefined) paths.push(full);
        if (paths.length) await b.delete(paths);
        return ok();
      }
      default:
        return fail("invalid-path: empty");
    }
  }
}
