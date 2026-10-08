// Each app runs in its own dedicated Worker (PRODUCTION R3, GAPS R1). The
// document keeps the DOM renderer and a `WorkerApp`, a proxy with the same
// surface as `PlinthApp` (`load`, `init`, `onEvent`, `onCommit`, `onStop`,
// `onError`, `stopped`). The Worker runs `PlinthApp` (the core and the app
// module) and talks to the proxy only through `postMessage`.
//
// Why: a Worker is the only place where the page can stop a computation
// that does not end (`Worker.terminate()`). The proxy has a watchdog: a
// call into the app (`init` or one event, also timers, frames and answers)
// that does not end in 2 s (`CALL_TIME_LIMIT_MS`, as on the desktop) ends
// the Worker, and the app stops with "the app is not responding (time
// limit reached)".
//
// The host imports that need the document cross the boundary as messages
// (GAPS "R1/R2 design"):
// - `ui.commit`: the op bytes go to the document (the buffer is
//   transferred, not copied); the proxy decodes them and calls `onCommit`.
// - `dev.log`, `error.report`: messages to the document.
// - `time.*` timers: in the Worker (`setTimeout`/`setInterval` exist there).
// - `time.set-frame-timer`: a Worker has no `requestAnimationFrame`, so the
//   Worker asks the document for each frame (`frame-request`), and the
//   document sends the frame time back (`frame`).
// - `store.kv`: the Worker answers from a snapshot that the document sends
//   at the start; each write goes to the document (the pattern of
//   `frameKvStore` in `hub-storage.js`).
// - `clipboard`: `write-text` goes to the document; `read-text` answers from
//   the cache of the Worker and asks the document for a new value.
// - `dialog`, `net.fetch`, `files.*`, `sql.run`: a request id at once, the
//   answer from the document later (`ask`/`answer`), as before.
// - `hub.*` (the Hub app only): `FrameHub` runs in the Worker on its
//   snapshot; its `hub-save`, `hub-launch` and `hub-call` messages go out
//   through the document.
//
// No DOM code: `test/run-worker.mjs` runs both sides in Node over a
// `MessageChannel`.

import { PlinthApp, httpFetch, defaultAskDialog, browserNotify, parseManifest, CALL_TIME_LIMIT_MS, TIME_LIMIT_REASON } from "./plinth-web.js";
import { decodeOps } from "./protocol.js";
import { frameKvStore } from "./hub-storage.js";
import { FileSpace, memoryFileBackend } from "./files.js";

/**
 * Starts the Worker of one app from a `blob:` URL, so that no second file
 * on disk is necessary and a sandboxed frame (an opaque origin) can start
 * it. The Worker is always a classic script: Edge refuses a module Worker
 * from a `blob:` URL in an opaque origin. `bundleScript`: the classic
 * script of the bundle `plinth.js` (it calls `runWorker` itself); else a
 * small script imports this file from `moduleUrl` (`import()` works in a
 * classic Worker) and keeps the messages that come before the import ends.
 */
export function startAppWorker(moduleUrl, bundleScript = null) {
  const source =
    bundleScript ??
    `"use strict";
  const early = [];
  self.onmessage = (event) => early.push(event);
  import(${JSON.stringify(moduleUrl)}).then(
    (m) => {
      m.runWorker(self);
      for (const event of early) self.onmessage(event);
    },
    (err) => postMessage({ t: "load-failed", message: "the app worker did not start: " + (err && err.message ? err.message : err), stack: String(err && err.stack ? err.stack : err) }),
  );
`;
  const url = URL.createObjectURL(new Blob([source], { type: "text/javascript" }));
  const worker = new Worker(url);
  const revoke = () => {
    URL.revokeObjectURL(url);
    worker.removeEventListener("message", revoke);
    worker.removeEventListener("error", revoke);
  };
  worker.addEventListener("message", revoke);
  worker.addEventListener("error", revoke);
  return worker;
}

/** A kv store on a snapshot that sends each write (no quota: the stand-alone page). */
function cachedKvStore(snapshot, send) {
  const map = new Map(Object.entries(snapshot ?? {}));
  return {
    get: (key) => (map.has(key) ? map.get(key) : null),
    set(key, value) {
      map.set(key, value);
      send({ type: "kv-set", key, value });
      return true;
    },
    delete(key) {
      if (!map.delete(key)) return;
      send({ type: "kv-delete", key });
    },
    keys: () => [...map.keys()],
  };
}

/**
 * `plinth:app-info` (core 1.13, PRODUCTION.md D4): the data version on a
 * single stored value (`initial`, from the document's `load` message),
 * sending each write to the document (`cachedKvStore`'s idea, but for one
 * value, since every app in a Worker has exactly one data version).
 */
function cachedDataVersionStore(initial, send) {
  let version = initial ?? null;
  return {
    get: () => version,
    set(_key, value) {
      version = value;
      send({ type: "data-version-set", version: value });
    },
  };
}

/**
 * The Worker side. `scope` is the global scope of the Worker (or one end
 * of a `MessageChannel` in a test): anything with `postMessage` and
 * `onmessage`.
 */
export function runWorker(scope) {
  let app = null;
  let hub = null;
  let frameCallback = null;
  let nextAsk = 1;
  const pending = new Map(); // ask id -> { resolve, reject }
  const post = (msg, transfer = []) => scope.postMessage(msg, transfer);
  const send = (msg) => post({ t: "send", msg });
  const ask = (type, fields) =>
    new Promise((resolve, reject) => {
      const id = nextAsk++;
      pending.set(id, { resolve, reject });
      post({ t: "ask", id, type, fields });
    });

  async function load(m) {
    if (m.hub) {
      // Only the Hub app gets a snapshot. A web export (plinth.js) never
      // does, so the bundle never loads this module.
      const { FrameHub } = await import("./hub-host.js");
      hub = new FrameHub(m.hub, { send, ask });
    }
    app = new PlinthApp();
    app.onError = (text) => post({ t: "error", text });
    app.onStop = (reason, err) => post({ t: "stop", reason, message: String(err?.message ?? err) });
    // The watchdog in the document (`WorkerApp`) times each call. A message
    // posted before an endless loop still reaches the document.
    app.onCallStart = () => post({ t: "call" });
    app.onCallEnd = () => post({ t: "done" });
    await app.load(m.core, m.app, {
      manifestText: m.manifestText,
      kvStore: typeof m.kvQuota === "number" ? frameKvStore(m.kv, m.kvQuota, send) : cachedKvStore(m.kv, send),
      // plinth:app-info (core 1.13, PRODUCTION.md D4): the document sent the
      // stored data version in `load` (`WorkerApp.load`); each write goes back.
      dataVersionStore: cachedDataVersionStore(m.dataVersion, send),
      refused: new Set(m.refused ?? []),
      memoryLimitPages: m.memoryLimitPages,
      // plinth:locale (core 1.13): the page's locale; else the Worker's own.
      locale: m.locale ?? undefined,
      log: (text) => post({ t: "log", text }),
      reportError: () => {}, // the document reports it (`error`)
      askDialog: (kind, message) => ask("dialog", { kind, message: String(message) }),
      clipboard: {
        writeText: (text) => send({ type: "clipboard-write", text }),
        readText: () => ask("clipboard-read", {}),
      },
      netFetch: (url, method, headers, body) => ask("net-fetch", { url, method, headers, body }),
      files: { call: (op, path, text) => ask(`files-${op}`, op === "write" ? { path, text } : { path }) },
      sql: { call: (kind, statement, params) => ask("sql-run", { kind, statement, params }) },
      notify: {
        show: (title, body, tag) => ask("notify-show", { title, body, tag }),
        cancel: (tag) => send({ type: "notify-cancel", tag }),
      },
      hub,
      requestFrame: (callback) => {
        frameCallback = callback;
        post({ t: "frame-request" });
        return 1;
      },
      cancelFrame: () => {
        frameCallback = null;
        post({ t: "frame-cancel" });
      },
      onCommitBytes: (bytes) => post({ t: "commit", bytes }, [bytes.buffer]),
    });
  }

  scope.onmessage = (event) => {
    const m = event.data;
    switch (m?.t) {
      case "load":
        load(m).then(
          () => post({ t: "loaded" }),
          (err) => post({ t: "load-failed", message: String(err?.message ?? err), stack: String(err?.stack ?? err) }),
        );
        break;
      case "init":
        app?.init(m.records);
        post({ t: "init-done" });
        break;
      case "event":
        app?.onEvent(m.event);
        break;
      case "frame": {
        const callback = frameCallback;
        frameCallback = null;
        callback?.(m.time);
        break;
      }
      case "answer": {
        const p = pending.get(m.id);
        if (!p) break;
        pending.delete(m.id);
        if (m.error !== undefined) p.reject(new Error(m.error));
        else p.resolve(m.value ?? null);
        break;
      }
      case "locale":
        app?.setLocale(m.tag);
        break;
      case "hub-state":
        try {
          hub?.applySnapshot({ state: m.state });
        } catch (err) {
          console.error(err);
        }
        break;
    }
  };
}

/** The grace after the time limit (ms): see `WorkerApp._callStarted`. */
const WATCHDOG_GRACE_MS = 100;

/**
 * The document side: a proxy with the surface of `PlinthApp`. `worker` is
 * a `Worker` (`startAppWorker`) or anything with `postMessage`,
 * `onmessage` and optionally `terminate`.
 */
export class WorkerApp {
  constructor(worker) {
    this.worker = worker;
    this.onCommit = () => {};
    /** The plain reason after a trap, else null (as `PlinthApp.stopped`). */
    this.stopped = null;
    /** `(reason, error)` after the app stopped. */
    this.onStop = null;
    /** `(message)` for each uncaught app error; the app keeps running. */
    this.onError = null;
    this.opts = {};
    this._loading = null; // { resolve, reject } while `load` runs
    this._inits = []; // resolves of the `init` calls, in order
    this._frame = null;
    /** The time limit of one call (ms); `load` can change it (`opts.timeLimitMs`, tests). */
    this.timeLimitMs = CALL_TIME_LIMIT_MS;
    this._watchdog = null; // the timer of the open call, else null
    this._dead = false; // the Worker was ended at the time limit
    worker.onmessage = (event) => this._receive(event.data);
    if (typeof worker.addEventListener === "function") {
      worker.addEventListener("error", (event) => {
        const message = event?.message || "the app worker did not start";
        if (this._loading) {
          event?.preventDefault?.();
          this._loading.reject(new Error(message));
          this._loading = null;
        } else console.error(`[plinth] app worker: ${message}`);
      });
    }
  }

  /**
   * Starts the core and the app in the Worker. `opts` (all optional):
   * `manifestText`; `kvStore` ({get,set,delete,keys}, the document's store:
   * the Worker gets a snapshot, and each write comes back to this store);
   * `kvQuota` (characters; the Worker refuses a write over it, as
   * `frameKvStore`); `dataVersionStore` ({get(key), set(key, value)}, core
   * 1.13, PRODUCTION.md D4, `onUpgrade`: the document's store for the data
   * version, the same shape as `plinth-web.js`'s default; the Worker gets
   * the stored value at the start and sends back each write); `refused`;
   * `log`; `reportError`; `askDialog`;
   * `clipboard`; `netFetch`; `files` (a `FileSpace`); `sql` (`{ call(kind,
   * statement, params) }`, the page's `store.sql` engine; none: an allowed
   * call answers "denied:unsupported"); `notify` (`{ show,
   * cancel }`; default `browserNotify()`, the page's `Notification` API);
   * `memoryLimitPages`; `locale` (a BCP 47 tag; default the Worker's
   * `navigator.languages`);
   * `timeLimitMs` (the time limit of one call; default `CALL_TIME_LIMIT_MS`);
   * for the Hub app: `hubSnapshot`, and `send(msg)`/`ask(type, fields)` to
   * the page for the `hub-*` messages.
   */
  load(coreBytes, appBytes, opts = {}) {
    this.opts = opts;
    if (typeof opts.timeLimitMs === "number") this.timeLimitMs = opts.timeLimitMs;
    this.files = opts.files ?? new FileSpace(memoryFileBackend(opts.filesMap));
    this.clip =
      opts.clipboard ??
      (typeof navigator !== "undefined" && navigator.clipboard?.writeText
        ? { writeText: (t) => navigator.clipboard.writeText(t), readText: () => navigator.clipboard.readText() }
        : null);
    const kv = {};
    if (opts.kvStore) for (const k of opts.kvStore.keys()) kv[k] = opts.kvStore.get(k);
    // `plinth:app-info` (core 1.13, PRODUCTION.md D4): the app id, so a
    // write of the data version from the Worker (`data-version-set`) goes
    // back to `opts.dataVersionStore` under the same key the store expects
    // (the same id `PlinthApp.load` uses inside the Worker).
    this._appId = opts.manifestText ? parseManifest(opts.manifestText).id : "";
    const dataVersion = opts.dataVersionStore ? opts.dataVersionStore.get(this._appId) : null;
    return new Promise((resolve, reject) => {
      this._loading = { resolve: () => resolve(this), reject };
      this._post({
        t: "load",
        core: coreBytes,
        app: appBytes,
        manifestText: opts.manifestText ?? "",
        kv,
        kvQuota: opts.kvQuota ?? null,
        refused: [...(opts.refused ?? [])],
        hub: opts.hubSnapshot ?? null,
        memoryLimitPages: opts.memoryLimitPages,
        locale: opts.locale ?? null,
        dataVersion,
      });
    });
  }

  /** Calls `init` in the Worker; the promise resolves after the call (and its commits). */
  init(initArgRecords = []) {
    return new Promise((resolve) => {
      this._inits.push(resolve);
      this._post({ t: "init", records: initArgRecords });
    });
  }

  onEvent(event) {
    if (this.stopped !== null) return;
    this._post({ t: "event", event });
  }

  /** plinth:locale (core 1.13): the locale of the app changed (`PlinthApp.setLocale`). */
  setLocale(tag) {
    this._post({ t: "locale", tag });
  }

  /** The Hub app only: the page changed the `plinth:hub` state. */
  applyHubState(state) {
    this._post({ t: "hub-state", state });
  }

  /** Ends the Worker. */
  terminate() {
    this._dead = true;
    clearTimeout(this._watchdog);
    this._watchdog = null;
    this._cancelFrame();
    this.worker.terminate?.();
  }

  _post(msg) {
    if (!this._dead) this.worker.postMessage(msg);
  }

  /**
   * The watchdog (GAPS R1). The Worker sends `call` before each call into
   * the app and `done` after it; the calls do not overlap. The time starts
   * when the document reads `call`, so a busy document makes the limit
   * longer, never shorter. At the limit, a short grace lets a `done` that
   * already waits in the queue arrive first (the document can be busy with
   * a large commit of the same call).
   */
  _callStarted() {
    if (this._dead || this.stopped !== null) return;
    clearTimeout(this._watchdog);
    this._watchdog = setTimeout(() => {
      this._watchdog = setTimeout(() => this._timeLimit(), WATCHDOG_GRACE_MS);
    }, this.timeLimitMs);
  }

  _callEnded() {
    clearTimeout(this._watchdog);
    this._watchdog = null;
  }

  /** The call did not end in time: end the Worker and stop the app, as the desktop does. */
  _timeLimit() {
    this._watchdog = null;
    if (this._dead || this.stopped !== null) return;
    this.terminate();
    this.stopped = TIME_LIMIT_REASON;
    const err = new Error(TIME_LIMIT_REASON);
    console.error(`[plinth] the app stopped: ${this.stopped}`);
    // An `init` that did not end: its caller goes on, and shows the banner.
    for (const resolve of this._inits.splice(0)) resolve();
    this.onStop?.(this.stopped, err);
  }

  _cancelFrame() {
    if (this._frame === null) return;
    (globalThis.cancelAnimationFrame?.bind(globalThis) ?? clearTimeout)(this._frame);
    this._frame = null;
  }

  _receive(m) {
    if (this._dead) return;
    switch (m?.t) {
      case "commit":
        this.onCommit(decodeOps(m.bytes));
        break;
      case "log":
        (this.opts.log ?? ((s) => console.log(s)))(m.text);
        break;
      case "error":
        (this.opts.reportError ?? ((s) => console.error(s)))(m.text);
        this.onError?.(m.text);
        break;
      case "stop":
        this.stopped = m.reason;
        if (this.onStop) this.onStop(m.reason, new Error(m.message));
        break;
      case "init-done":
        this._inits.shift()?.();
        break;
      case "call":
        this._callStarted();
        break;
      case "done":
        this._callEnded();
        break;
      case "loaded":
        this._loading?.resolve();
        this._loading = null;
        break;
      case "load-failed": {
        const err = new Error(m.message);
        err.stack = m.stack;
        this._loading?.reject(err);
        this._loading = null;
        break;
      }
      case "frame-request":
        if (this._frame === null) {
          const raf = globalThis.requestAnimationFrame?.bind(globalThis) ?? ((cb) => setTimeout(() => cb(performance.now()), 16));
          this._frame = raf((time) => {
            this._frame = null;
            this._post({ t: "frame", time });
          });
        }
        break;
      case "frame-cancel":
        this._cancelFrame();
        break;
      case "send":
        this._send(m.msg);
        break;
      case "ask":
        Promise.resolve()
          .then(() => this._ask(m.type, m.fields ?? {}))
          .then(
            (value) => this._post({ t: "answer", id: m.id, value: value ?? null }),
            (err) => this._post({ t: "answer", id: m.id, error: String(err?.message ?? err) }),
          );
        break;
    }
  }

  /** A fire-and-forget message from the Worker. */
  _send(msg) {
    const o = this.opts;
    switch (msg?.type) {
      case "kv-set":
        o.kvStore?.set(msg.key, msg.value);
        break;
      case "kv-delete":
        o.kvStore?.delete(msg.key);
        break;
      case "data-version-set":
        o.dataVersionStore?.set(this._appId, msg.version);
        break;
      case "clipboard-write":
        if (this.clip) Promise.resolve().then(() => this.clip.writeText(msg.text)).catch(() => {});
        break;
      case "notify-cancel":
        Promise.resolve()
          .then(() => this._notify().cancel(msg.tag))
          .catch(() => {});
        break;
      case "hub-save":
      case "hub-launch":
        o.send?.(msg);
        break;
    }
  }

  /** The `plinth:notify` backend: the caller's, else the page's `Notification` API. */
  _notify() {
    this.notifyBackend ??= this.opts.notify ?? browserNotify();
    return this.notifyBackend;
  }

  /** A request from the Worker; the result is its answer. */
  _ask(type, f) {
    const o = this.opts;
    if (type === "dialog") return (o.askDialog ?? defaultAskDialog)(f.kind, f.message);
    if (type === "clipboard-read") return this.clip ? this.clip.readText() : null;
    if (type === "net-fetch") return o.netFetch ? o.netFetch(f.url, f.method, f.headers, f.body) : httpFetch(f.url, f.method, f.headers, f.body);
    if (type === "notify-show") return this._notify().show(f.title, f.body ?? null, f.tag ?? null);
    if (type.startsWith("files-")) return this.files.call(type.slice("files-".length), f.path, f.text ?? null);
    if (type === "sql-run") return o.sql ? o.sql.call(f.kind, f.statement, f.params) : [false, 0, "", "denied:unsupported"];
    if (type === "hub-call" && o.ask) return o.ask(type, f);
    throw new Error(`unknown request ${type}`);
  }
}
