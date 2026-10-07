// The Plinth web host (SPEC.md §18.3 "browse mode" / side-module model).
//
// Loads the SAME `.plnt` and the SAME runtime core file the desktop host
// uses, and runs them in a browser (or Node 22, which has the same
// WebAssembly API). No static linking: the core (`plinth-rt`) is
// instantiated as a normal wasm module that exports its memory, table and
// every `__plinth_rt_*` function; `app.wasm` is instantiated as a second
// module that imports those exports from a module named `plinth-rt` (SPEC
// §10.4), after the host grows the core's table by the app's own table
// size and passes `table_base` as the growth point.
//
// This file has NO DOM code, so the same code runs in Node tests
// (web/test/*.mjs) and in the browser (dom-renderer.js consumes its
// `PlinthApp` events instead).

import { openZip } from "./zip.js";
import { decodeOps, encodeEvent, encodeInitArgs } from "./protocol.js";
import { FileSpace, memoryFileBackend, FILES_CAPABILITY } from "./files.js";

const RT_PREFIX = "__plinth_rt_";

/** Reads the `plinth-rt`/`table` import's minimum size from a raw wasm module. */
function readTableImportMin(bytes) {
  let pos = 8; // past the 8-byte wasm header
  function u8() {
    return bytes[pos++];
  }
  function leb() {
    let result = 0,
      shift = 0,
      byte;
    do {
      byte = u8();
      result |= (byte & 0x7f) << shift;
      shift += 7;
    } while (byte & 0x80);
    return result >>> 0;
  }
  function str() {
    const len = leb();
    const s = new TextDecoder().decode(bytes.subarray(pos, pos + len));
    pos += len;
    return s;
  }
  while (pos < bytes.length) {
    const sectionId = u8();
    const sectionLen = leb();
    const sectionEnd = pos + sectionLen;
    if (sectionId === 2) {
      // import section
      const count = leb();
      for (let i = 0; i < count; i++) {
        const mod = str();
        const field = str();
        const kind = u8();
        if (kind === 0) {
          leb(); // type index
        } else if (kind === 1) {
          u8(); // elem type
          const flags = u8();
          const min = leb();
          if (flags & 1) leb(); // max
          if (mod === "plinth-rt" && field === "table") return min;
        } else if (kind === 2) {
          const flags = u8();
          leb(); // min
          if (flags & 1) leb(); // max
        } else if (kind === 3) {
          u8(); // val type
          u8(); // mutability
        }
      }
      return 0;
    }
    pos = sectionEnd;
  }
  return 0;
}

/** Reads `name` (manifest.toml, app.wasm) out of a fetched `.plnt` zip. */
export async function readPlnt(bytes) {
  const zip = openZip(bytes);
  const manifestText = new TextDecoder().decode(await zip.read("manifest.toml"));
  const appWasm = await zip.read(zip.has("app.wasm") ? "app.wasm" : manifestEntry(manifestText));
  // Assets (SPEC.md §10.1), by path under `assets/` without the prefix, for
  // `<Image>`. Read eagerly: packages are small and this keeps the DOM
  // renderer synchronous.
  const assets = new Map();
  for (const name of zip.names()) {
    if (name.startsWith("assets/")) assets.set(name.slice("assets/".length), await zip.read(name));
  }
  return { manifestText, appWasm, assets };
}

function manifestEntry(manifestText) {
  const m = manifestText.match(/entry\s*=\s*"([^"]+)"/);
  return m ? m[1] : "app.wasm";
}

const DeniedReason = { undeclared: 0, refused: 1, unsupported: 2 };

/** True for a host in a private or loopback range (`net.local`, SPEC.md §11). */
export function isPrivateNetHost(host) {
  const h = host.split(":")[0]; // strip a port
  if (h.toLowerCase() === "localhost" || h.endsWith(".local") || h === "::1" || h === "0.0.0.0" || h === "[::1]") return true;
  const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!m) return false;
  const [a, b] = [Number(m[1]), Number(m[2])];
  return a === 127 || a === 10 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168);
}

/**
 * The `plinth:net` check (SPEC.md §11): `net:<host>` and `net.local` are
 * dynamic names, so the check runs against the URL. `capReason(name)` gives
 * null (usable) or a `DeniedReason`. Returns null or a `DeniedReason`.
 */
function netDeniedReason(url, capReason) {
  let host;
  try {
    host = new URL(url).hostname;
  } catch {
    return DeniedReason.unsupported;
  }
  const names = isPrivateNetHost(host) ? ["net.local"] : [`net:${host}`, "net:*"];
  const reasons = names.map(capReason);
  if (reasons.includes(null)) return null;
  return reasons.includes(DeniedReason.refused) ? DeniedReason.refused : DeniedReason.undeclared;
}

/**
 * The `plinth:net` check for a host page (the app-frame host checks the
 * request again in the parent): null if `url` may be fetched, else the
 * `"denied:<reason>"` text. `declared` and `refused` are capability sets.
 */
export function netDenied(url, declared, refused = new Set()) {
  const capReason = (name) => (!declared.has(name) ? DeniedReason.undeclared : refused.has(name) ? DeniedReason.refused : null);
  const reason = netDeniedReason(url, capReason);
  if (reason === null) return null;
  return reason === DeniedReason.undeclared ? "denied:undeclared" : reason === DeniedReason.refused ? "denied:refused" : "denied:unsupported";
}

/** One HTTP request for `plinth:net`: `[ok, status, text, error]`, never a throw. */
export async function httpFetch(url, method, headers, body, impl = typeof fetch === "function" ? fetch : null) {
  if (!impl) return [false, 0, "", "network: no fetch implementation available"];
  const controller = typeof AbortController === "function" ? new AbortController() : null;
  const timeout = controller ? setTimeout(() => controller.abort(), 20000) : null;
  try {
    const res = await impl(url, {
      method,
      headers,
      body: body ?? undefined,
      redirect: "manual", // redirects are allowed only to allowed hosts (SPEC.md §11); not re-checked per hop yet.
      credentials: "omit", // an app request never carries the page's cookies
      signal: controller?.signal,
    });
    const text = await res.text();
    return [res.ok, res.status, text, null];
  } catch (e) {
    return [false, 0, "", `network: ${e?.message ?? e}`];
  } finally {
    if (timeout) clearTimeout(timeout);
  }
}

/**
 * A `Map`-backed implementation of the kv storage interface `hostImports`
 * expects ({get,set,delete,keys}, all synchronous). Used as the Node
 * default (SPEC.md §18.3: no DOM, so no `localStorage`); two `PlinthApp`s
 * given the SAME `Map` see each other's writes, same as two tabs sharing
 * `localStorage`.
 */
export function mapKvStore(map = new Map()) {
  return {
    get(key) {
      return map.has(key) ? map.get(key) : null;
    },
    set(key, value) {
      map.set(key, value);
    },
    delete(key) {
      map.delete(key);
    },
    keys() {
      return [...map.keys()];
    },
  };
}

/**
 * A `localStorage`-backed implementation of the kv storage interface, one
 * key prefix per app id (so several installed/browsed apps sharing one
 * `localStorage` origin do not collide, SPEC.md §18).
 */
export function localStorageKvStore(storage, prefix) {
  return {
    get(key) {
      return storage.getItem(prefix + key);
    },
    set(key, value) {
      storage.setItem(prefix + key, value);
    },
    delete(key) {
      storage.removeItem(prefix + key);
    },
    keys() {
      const out = [];
      for (let i = 0; i < storage.length; i++) {
        const k = storage.key(i);
        if (k && k.startsWith(prefix)) out.push(k.slice(prefix.length));
      }
      return out;
    },
  };
}

/** Parses just enough of a `.plnt` manifest to get the app id and declared capability names. */
export function parseManifest(manifestText) {
  const idMatch = manifestText.match(/^id\s*=\s*"([^"]*)"/m);
  const capabilities = new Set();
  const capRe = /\[\[capabilities\]\]\s*\r?\n\s*name\s*=\s*"([^"]*)"/g;
  let m;
  while ((m = capRe.exec(manifestText))) capabilities.add(m[1]);
  const nameMatch = manifestText.match(/^name\s*=\s*"([^"]*)"/m);
  const runtimeMatch = manifestText.match(/^runtime\s*=\s*"([^"]*)"/m);
  return { id: idMatch ? idMatch[1] : "", name: nameMatch ? nameMatch[1] : "", runtime: runtimeMatch ? runtimeMatch[1] : "", capabilities };
}

/**
 * Builds the JS implementations of the core's WIT-level host imports
 * (`plinth:app/{ui,dev,time,store,clipboard}@1.0.0`, SPEC.md §8.5). `onCommit`
 * is called with a decoded op list for every `ui.commit`. `log` defaults to
 * `console.log`.
 *
 * `store.kv` is backed by `kvStore` ({get,set,delete,keys}, synchronous),
 * gated on the manifest declaring the `store.kv` capability (`capabilities`,
 * a `Set<string>`): when it is not declared every call returns
 * `denied(undeclared)`, same shape a host with no grant returns (SPEC.md
 * §11). `clipboard` backs onto `navigator.clipboard`: its APIs are async
 * and this call is sync, so `writeText` is fire-and-forget (it also updates
 * a local cache immediately) and `readText` returns the cached value (and
 * kicks off an async read to refresh the cache for next time). Timers are
 * real (JS `setTimeout`/`setInterval`), driven by `scheduleTimerEvent`.
 */
/**
 * The default dialog answers: the browser's own dialogs when they exist
 * (`alert` → null, `confirm` → boolean, `prompt` → string or null), else
 * (Node) a declined answer.
 */
function defaultAskDialog(kind, message) {
  const g = globalThis;
  if (kind === "alert") {
    if (typeof g.alert === "function") g.alert(message);
    return null;
  }
  if (kind === "confirm") return typeof g.confirm === "function" ? Boolean(g.confirm(message)) : false;
  return typeof g.prompt === "function" ? g.prompt(message) : null;
}

function hostImports(
  getExports,
  onCommit,
  {
    log = (s) => console.log(s),
    reportError = (s) => console.error(s),
    scheduleTimerEvent,
    cancelTimerEvent,
    kvStore,
    capabilities = new Set(),
    refused = new Set(),
    askDialog,
    completeRequest,
    clipboard = null,
    hub = null,
    netFetch = null,
    files = null,
  } = {},
) {
  // A capability is usable when the manifest declares it and the user did
  // not refuse it (the web App Hub's consent, docs/web-hub.md). A refused
  // call answers `denied(refused)`; it never traps (SPEC.md §8.5).
  function capReason(name) {
    if (!capabilities.has(name)) return DeniedReason.undeclared;
    return refused.has(name) ? DeniedReason.refused : null;
  }
  // plinth:dialog (core 1.3, SPEC.md §8.5): each call returns a request id at
  // once; the answer arrives later as a `completion` event.
  let nextRequest = 1;
  function openDialog(kind, message) {
    const id = nextRequest++;
    const ask = askDialog ?? defaultAskDialog;
    Promise.resolve()
      .then(() => ask(kind, message))
      .then((answer) => completeRequest?.(id, answer ?? null));
    return id;
  }
  // plinth:net (core 1.4, SPEC.md §8.5, §11): same request-id-now,
  // completion-later shape as dialogs. The check is `netDeniedReason`
  // (module level, shared with the app-frame host).
  function openNetFetch(url, method, headers, body) {
    const id = nextRequest++;
    const reason = netDeniedReason(url, capReason);
    if (reason !== null) {
      const text = reason === DeniedReason.undeclared ? "denied:undeclared" : reason === DeniedReason.refused ? "denied:refused" : "denied:unsupported";
      Promise.resolve().then(() => completeRequest?.(id, [false, 0, "", text]));
      return id;
    }
    // `netFetch` (the app frame): the parent page makes the request, so it
    // carries the page's real origin (docs/web-hub.md §4). It answers the
    // same `[ok, status, text, error]` result.
    const run = netFetch
      ? Promise.resolve()
          .then(() => netFetch(url, method, headers, body))
          .catch((e) => [false, 0, "", `network: ${e?.message ?? e}`])
      : httpFetch(url, method, headers, body);
    run.then((result) => completeRequest?.(id, result));
    return id;
  }
  // plinth:files (core 1.11, docs/STORAGE.md §2, §3): the same request id
  // and `[ok, 0, text, error]` completion as `net.fetch`. `files` is a
  // `FileSpace` (or anything with its `call(op, path, text)`): in memory by
  // default, the page's store in a sandboxed frame. It checks each path
  // again; the frame's parent checks the capability again.
  function openFiles(op, path, text) {
    const id = nextRequest++;
    let reason = capReason(FILES_CAPABILITY);
    if (reason === null && !files) reason = DeniedReason.unsupported;
    const run =
      reason !== null
        ? Promise.resolve([false, 0, "", deniedText(reason)])
        : Promise.resolve()
            .then(() => files.call(op, path, text))
            .catch((e) => [false, 0, "", `io: ${e?.message ?? e}`]);
    run.then((result) => completeRequest?.(id, result));
    return id;
  }
  function mem() {
    return new DataView(getExports().memory.buffer);
  }
  function bytes() {
    return new Uint8Array(getExports().memory.buffer);
  }
  function readString(ptr, len) {
    return new TextDecoder().decode(bytes().subarray(ptr, ptr + len));
  }
  function alloc(size) {
    return getExports().cabi_realloc(0, 0, 1, size || 1);
  }
  function writeString(s) {
    const encoded = new TextEncoder().encode(s);
    const ptr = alloc(encoded.length || 1);
    bytes().set(encoded, ptr);
    return { ptr, len: encoded.length };
  }
  // result<_, host-error>: tag@0 (1 byte), err payload (host-error: tag@1, reason@2).
  function writeDeniedUnit(retptr, reason = DeniedReason.unsupported) {
    const v = mem();
    v.setUint8(retptr, 1);
    v.setUint8(retptr + 1, 0);
    v.setUint8(retptr + 2, reason);
  }
  function writeOkUnit(retptr) {
    mem().setUint8(retptr, 0);
  }
  // result<T, host-error> where T contains an i32 (option<string>, list<string>, u32):
  // outer align is 4, so tag@0 and the payload (ok or err) start at offset 4.
  function writeDeniedAt4(retptr, reason = DeniedReason.unsupported) {
    const v = mem();
    v.setUint8(retptr, 1);
    v.setUint8(retptr + 4, 0);
    v.setUint8(retptr + 5, reason);
  }
  // Ok(option<string>): tag@0=0, option-tag@4 (0=none,1=some), string ptr@8/len@12 if some.
  function writeOkOptionString(retptr, value) {
    const v = mem();
    v.setUint8(retptr, 0);
    if (value === null || value === undefined) {
      v.setUint8(retptr + 4, 0);
      return;
    }
    v.setUint8(retptr + 4, 1);
    const { ptr, len } = writeString(value);
    v.setUint32(retptr + 8, ptr, true);
    v.setUint32(retptr + 12, len, true);
  }
  // Ok(string): tag@0=0, string ptr@4/len@8 (same layout as option<string>'s
  // "some" case minus the option tag: a plain string's natural alignment is
  // already 4, so there is no extra tag byte to skip).
  function writeOkString(retptr, value) {
    const v = mem();
    v.setUint8(retptr, 0);
    const { ptr, len } = writeString(value);
    v.setUint32(retptr + 4, ptr, true);
    v.setUint32(retptr + 8, len, true);
  }
  // Ok(list<string>): tag@0=0, list ptr@4/len@8; each element is (ptr: i32, len: i32).
  function writeOkStringList(retptr, values) {
    const v = mem();
    v.setUint8(retptr, 0);
    const arrPtr = alloc(Math.max(values.length, 1) * 8);
    for (let i = 0; i < values.length; i++) {
      const { ptr, len } = writeString(values[i]);
      v.setUint32(arrPtr + i * 8, ptr, true);
      v.setUint32(arrPtr + i * 8 + 4, len, true);
    }
    v.setUint32(retptr + 4, arrPtr, true);
    v.setUint32(retptr + 8, values.length, true);
  }

  // plinth:hub: the reason a call is denied, or null.
  function hubReason() {
    const reason = capReason("hub.manage");
    if (reason !== null) return reason;
    return hub ? null : DeniedReason.unsupported;
  }
  function hubString(retptr, call) {
    const reason = hubReason();
    if (reason !== null) return writeDeniedAt4(retptr, reason);
    let value;
    try {
      value = call();
    } catch {
      return writeDeniedAt4(retptr, DeniedReason.unsupported);
    }
    writeOkString(retptr, value);
  }
  function hubUnit(retptr, call) {
    const reason = hubReason();
    if (reason !== null) return writeDeniedUnit(retptr, reason);
    try {
      call();
    } catch {
      return writeDeniedUnit(retptr, DeniedReason.unsupported);
    }
    writeOkUnit(retptr);
  }
  const deniedText = (reason) =>
    reason === DeniedReason.undeclared ? "denied:undeclared" : reason === DeniedReason.refused ? "denied:refused" : "denied:unsupported";
  /** An async hub call: `denied` is null (answer null) or "text" (answer the reason text). */
  function hubAsync(denied, call, ok, failed) {
    const id = nextRequest++;
    const reason = hubReason();
    if (reason !== null) {
      Promise.resolve().then(() => completeRequest?.(id, denied === "text" ? deniedText(reason) : null));
      return id;
    }
    Promise.resolve()
      .then(call)
      .then(
        (value) => completeRequest?.(id, ok(value)),
        (err) => completeRequest?.(id, failed(String(err?.message ?? err))),
      );
    return id;
  }

  let nextTimer = 1;
  const timers = new Map(); // id -> JS handle
  const storeReason = kvStore ? capReason("store.kv") : DeniedReason.undeclared;
  let clipboardCache = null;
  // The clipboard: `navigator.clipboard` by default; the web App Hub's
  // sandboxed frame gives its own (`{ writeText(text), readText() }`, both
  // may return a promise) that asks the hub page.
  const clip =
    clipboard ??
    (typeof navigator !== "undefined" && navigator.clipboard?.writeText
      ? { writeText: (t) => navigator.clipboard.writeText(t), readText: () => navigator.clipboard.readText() }
      : null);

  return {
    "plinth:app/ui@1.0.0": {
      commit(ptr, len) {
        const copy = bytes().slice(ptr, ptr + len);
        onCommit(decodeOps(copy));
      },
    },
    "plinth:app/dev@1.0.0": {
      log(ptr, len) {
        log(readString(ptr, len));
      },
    },
    // Core 1.10 (SPEC.md §5.6): an error the app did not catch. The app
    // keeps running; `reportError` defaults to `console.error`.
    "plinth:app/error@1.0.0": {
      report(ptr, len) {
        reportError(readString(ptr, len));
      },
    },
    "plinth:app/time@1.0.0": {
      now() {
        return BigInt(Date.now());
      },
      "monotonic-now"() {
        return BigInt(Math.round(performance.now()));
      },
      "set-timer"(ms, repeat, retptr) {
        if (!scheduleTimerEvent) {
          writeDeniedAt4(retptr);
          return;
        }
        const id = nextTimer++;
        const fire = () => scheduleTimerEvent(id);
        const handle = repeat ? setInterval(fire, ms) : setTimeout(fire, ms);
        timers.set(id, { handle, repeat: !!repeat });
        const v = mem();
        v.setUint8(retptr, 0); // ok
        v.setUint32(retptr + 4, id, true);
      },
      "cancel-timer"(id) {
        const t = timers.get(id);
        if (!t) return;
        if (t.repeat) clearInterval(t.handle);
        else clearTimeout(t.handle);
        timers.delete(id);
        cancelTimerEvent?.(id);
      },
      "timezone-offset"(ms) {
        // JS `getTimezoneOffset` is minutes *west* of UTC (backwards from
        // the WIT function, which is minutes east, matching Howard
        // Hinnant's/most other APIs' convention).
        return -new Date(Number(ms)).getTimezoneOffset();
      },
    },
    "plinth:app/store@1.0.0": {
      "kv-get"(keyPtr, keyLen, retptr) {
        if (storeReason !== null) return writeDeniedAt4(retptr, storeReason);
        writeOkOptionString(retptr, kvStore.get(readString(keyPtr, keyLen)));
      },
      "kv-set"(keyPtr, keyLen, valPtr, valLen, retptr) {
        if (storeReason !== null) return writeDeniedUnit(retptr, storeReason);
        // A store can refuse a write (`false`), for example over its quota.
        if (kvStore.set(readString(keyPtr, keyLen), readString(valPtr, valLen)) === false) return writeDeniedUnit(retptr, DeniedReason.refused);
        writeOkUnit(retptr);
      },
      "kv-delete"(keyPtr, keyLen, retptr) {
        if (storeReason !== null) return writeDeniedUnit(retptr, storeReason);
        kvStore.delete(readString(keyPtr, keyLen));
        writeOkUnit(retptr);
      },
      "kv-keys"(retptr) {
        if (storeReason !== null) return writeDeniedAt4(retptr, storeReason);
        writeOkStringList(retptr, kvStore.keys());
      },
    },
    "plinth:app/dialog@1.0.0": {
      alert(ptr, len) {
        return openDialog("alert", readString(ptr, len));
      },
      confirm(ptr, len) {
        return openDialog("confirm", readString(ptr, len));
      },
      prompt(ptr, len) {
        return openDialog("prompt", readString(ptr, len));
      },
    },
    "plinth:app/net@1.0.0": {
      // Lowered core ABI (checked against the built core with a binary
      // import-section dump): url(ptr,len), method(ptr,len),
      // headers(ptr,count) — each element a 16-byte (ptr,len,ptr,len)
      // tuple<string,string> record — body(tag,ptr,len) (an
      // `option<string>`, tag 0 = none). Returns the request id directly.
      fetch(urlPtr, urlLen, methodPtr, methodLen, headersPtr, headersCount, bodyTag, bodyPtr, bodyLen) {
        const url = readString(urlPtr, urlLen);
        const method = readString(methodPtr, methodLen);
        const v = mem();
        const headers = [];
        for (let i = 0; i < headersCount; i++) {
          const base = headersPtr + i * 16;
          const kPtr = v.getUint32(base, true);
          const kLen = v.getUint32(base + 4, true);
          const vPtr = v.getUint32(base + 8, true);
          const vLen = v.getUint32(base + 12, true);
          headers.push([readString(kPtr, kLen), readString(vPtr, vLen)]);
        }
        const body = bodyTag === 0 ? null : readString(bodyPtr, bodyLen);
        return openNetFetch(url, method, headers, body);
      },
    },
    // Core 1.11: each call returns a request id directly (string params are
    // (ptr, len) pairs).
    "plinth:app/files@1.0.0": {
      read(ptr, len) {
        return openFiles("read", readString(ptr, len), null);
      },
      write(ptr, len, textPtr, textLen) {
        return openFiles("write", readString(ptr, len), readString(textPtr, textLen));
      },
      "list-dir"(ptr, len) {
        return openFiles("list", readString(ptr, len), null);
      },
      stat(ptr, len) {
        return openFiles("stat", readString(ptr, len), null);
      },
      remove(ptr, len) {
        return openFiles("remove", readString(ptr, len), null);
      },
    },
    "plinth:app/clipboard@1.0.0": {
      "write-text"(ptr, len, retptr) {
        const reason = capReason("clipboard.write");
        if (reason !== null) return writeDeniedUnit(retptr, reason);
        const text = readString(ptr, len);
        clipboardCache = text;
        if (clip) {
          // Fire-and-forget: the Clipboard API is async, this host call is not.
          Promise.resolve()
            .then(() => clip.writeText(text))
            .catch(() => {});
        }
        writeOkUnit(retptr);
      },
      "read-text"(retptr) {
        const reason = capReason("clipboard.read");
        if (reason !== null) return writeDeniedAt4(retptr, reason);
        if (clip) {
          // Kick off a refresh for next time; this call answers from the cache now.
          Promise.resolve()
            .then(() => clip.readText())
            .then((v) => {
              if (typeof v === "string") clipboardCache = v;
            })
            .catch(() => {});
        }
        writeOkOptionString(retptr, clipboardCache);
      },
    },
    // `plinth:hub` (`docs/HUB.md` §4.1, §12.2): privileged. The calls go to
    // `hub` (`hub-host.js`), which only the web App Hub shell gives, and
    // only to the Hub app signed by a trusted key (`hub-shell.js`). Without
    // it every call answers "unsupported" (SPEC.md §9.4); a declared but
    // refused `hub.manage` answers "refused". Never a trap. A backend error
    // answers "unsupported", as on the desktop host.
    "plinth:app/hub@1.0.0": {
      "list-apps"(retptr) {
        hubString(retptr, () => hub.listAppsJson());
      },
      launch(idPtr, idLen, retptr) {
        hubUnit(retptr, () => hub.launch(readString(idPtr, idLen)));
      },
      "set-grant"(idPtr, idLen, capPtr, capLen, allowed, retptr) {
        hubUnit(retptr, () => hub.setGrant(readString(idPtr, idLen), readString(capPtr, capLen), Boolean(allowed)));
      },
      block(idPtr, idLen, retptr) {
        hubUnit(retptr, () => hub.block(readString(idPtr, idLen)));
      },
      unblock(idPtr, idLen, retptr) {
        hubUnit(retptr, () => hub.unblock(readString(idPtr, idLen)));
      },
      // Core 1.8 (`docs/HUB.md` §9.1, §5.2, H3 step 2).
      "list-groups"(retptr) {
        hubString(retptr, () => hub.listGroupsJson());
      },
      "create-group"(namePtr, nameLen, retptr) {
        hubUnit(retptr, () => hub.createGroup(readString(namePtr, nameLen)));
      },
      "set-group"(idPtr, idLen, groupPtr, groupLen, member, retptr) {
        hubUnit(retptr, () => hub.setGroup(readString(idPtr, idLen), readString(groupPtr, groupLen), Boolean(member)));
      },
      remove(idPtr, idLen, retptr) {
        hubUnit(retptr, () => hub.remove(readString(idPtr, idLen)));
      },
      // Async: a request id now, the completion later (`wit/plinth/app.wit`).
      // Denied: null for `search` and `check-updates`, the reason text for
      // `install` and `update`.
      search(queryPtr, queryLen) {
        const query = readString(queryPtr, queryLen);
        return hubAsync(null, () => hub.search(query), (json) => json, () => null);
      },
      install(idPtr, idLen) {
        const app = readString(idPtr, idLen);
        return hubAsync("text", () => hub.install(app), () => null, (e) => e);
      },
      // Core 1.9 (`docs/HUB.md` §4.1, §7.4, §9.2).
      "app-info"(idPtr, idLen, retptr) {
        hubString(retptr, () => hub.appInfoJson(readString(idPtr, idLen)));
      },
      pin(idPtr, idLen, versionPtr, versionLen, retptr) {
        hubUnit(retptr, () => hub.pin(readString(idPtr, idLen), readString(versionPtr, versionLen)));
      },
      "block-publisher"(keyPtr, keyLen, retptr) {
        hubUnit(retptr, () => hub.blockPublisher(readString(keyPtr, keyLen)));
      },
      "unblock-publisher"(keyPtr, keyLen, retptr) {
        hubUnit(retptr, () => hub.unblockPublisher(readString(keyPtr, keyLen)));
      },
      "check-updates"(idPtr, idLen) {
        const app = readString(idPtr, idLen);
        // A failed check still answers with JSON, so the app can tell
        // "denied" (null) from "failed" (as the desktop host does).
        return hubAsync(
          null,
          () => hub.checkUpdates(app),
          (json) => json,
          (e) => JSON.stringify({ updates: [], errors: [e] }),
        );
      },
      update(idPtr, idLen) {
        const app = readString(idPtr, idLen);
        return hubAsync("text", () => hub.update(app), () => null, (e) => e);
      },
    },
  };
}

/**
 * Loads a core module (SPEC.md §10.5), ready to link one or more app
 * modules into. Exports `memory`, `table`, `cabi_realloc`, `init`,
 * `on-event`, and every `__plinth_rt_*` function. The table is exported
 * under whatever name wasm-ld gave it (normally `__indirect_function_table`).
 */
export async function loadCore(coreWasmBytes, opts = {}) {
  let instanceRef;
  const imports = hostImports(() => instanceRef.exports, opts.onCommit ?? (() => {}), opts);
  const { instance } = await WebAssembly.instantiate(coreWasmBytes, imports);
  instanceRef = instance;
  const tableExportName = Object.keys(instance.exports).find(
    (k) => instance.exports[k] instanceof WebAssembly.Table,
  );
  if (!tableExportName) throw new Error("the core does not export its function table (needs --export-table)");
  return { instance, tableExportName };
}

/**
 * Links `app.wasm` against an already-loaded core, side-module style (no
 * static linking, SPEC.md §18.3): grows the core's table by the app's own
 * table-import minimum, and imports the grown table, the core's memory and
 * every `__plinth_rt_*` export (without the prefix) under the module name
 * `plinth-rt`.
 */
export async function linkApp(core, appWasmBytes) {
  const { instance } = core;
  const table = instance.exports[core.tableExportName];
  const tableBaseValue = table.length;

  // Read the app module's own table import minimum, so we grow by exactly
  // that (wasm-encoder's split::encode_app writes one table import named
  // "table" with that minimum; WebAssembly.Module imports let us read it
  // without hand-parsing the binary).
  const rawBytes = appWasmBytes instanceof WebAssembly.Module ? null : appWasmBytes;
  const appModule = rawBytes ? await WebAssembly.compile(rawBytes) : appWasmBytes;
  // The JS `WebAssembly.Module.imports()` API does not expose a table
  // import's minimum size, so read it straight out of the binary's import
  // section (needed because app code puts closures in the table at
  // table-relative indices; SPEC.md §10.4).
  const tableMin = rawBytes ? readTableImportMin(rawBytes) : 0;
  if (tableMin > 0) table.grow(tableMin);

  const rt = {};
  for (const [name, fn] of Object.entries(instance.exports)) {
    if (name.startsWith(RT_PREFIX) && typeof fn === "function") {
      rt[name.slice(RT_PREFIX.length)] = fn;
    }
  }
  rt.memory = instance.exports.memory;
  rt.table = table;
  rt.table_base = new WebAssembly.Global({ value: "i32", mutable: false }, tableBaseValue);

  const appInstance = await WebAssembly.instantiate(appModule, { "plinth-rt": rt });
  return appInstance;
}

/**
 * The high-level facade: loads a `.plnt` and a core, links them, and
 * exposes `init`/`onEvent`. One `onCommit(ops)` callback per commit (there
 * may be more than one per `init`/`on-event` call). This is the object
 * `dom-renderer.js` and the node tests both drive.
 */
export class PlinthApp {
  constructor() {
    this.core = null;
    this.onCommit = () => {};
  }

  /**
   * `corePromiseOrBytes` and `appPromiseOrBytes` are `Uint8Array`s (already
   * fetched). Unless the caller supplies its own `scheduleTimerEvent`, a
   * fired timer is delivered back into the app as a `timer` event (SPEC.md
   * §8.4 code 0x03) through `on-event`, same as any other host event.
   *
   * `opts.manifestText` (the `.plnt`'s `manifest.toml`, from `readPlnt`) is
   * parsed for the app id and declared capabilities, so `store.kv` is only
   * granted when the manifest declares it (SPEC.md §11). `opts.kvStore`
   * overrides the backing store directly; otherwise one is built from
   * `opts.storage` (a `localStorage`-shaped object) keyed by the app id, or
   * an in-memory `Map` (`mapKvStore()`) when no `storage` is given — the
   * Node default, and also handy for sharing storage between two `PlinthApp`
   * instances in a test.
   */
  async load(coreBytes, appBytes, opts = {}) {
    const scheduleTimerEvent = opts.scheduleTimerEvent ?? ((id) => this.onEvent({ kind: "timer", timer: id }));
    const completeRequest = (id, result) => this.onEvent({ kind: "completion", request: id, result });
    const { id: appId, capabilities } = opts.manifestText ? parseManifest(opts.manifestText) : { id: "", capabilities: new Set() };
    const kvStore =
      opts.kvStore ?? (opts.storage ? localStorageKvStore(opts.storage, `plinth:${appId}:`) : mapKvStore(opts.sharedMap));
    this.core = await loadCore(coreBytes, {
      ...opts,
      scheduleTimerEvent,
      completeRequest,
      capabilities,
      refused: opts.refused ?? new Set(),
      hub: opts.hub ?? null,
      kvStore,
      // `plinth:files`: the caller's space (the frame's bridge to the page),
      // else a space in memory for this app (Node, the stand-alone page).
      files: opts.files ?? new FileSpace(memoryFileBackend(opts.filesMap)),
      onCommit: (ops) => this.onCommit(ops),
    });
    this.appInstance = await linkApp(this.core, appBytes);
    return this;
  }

  /** Allocates `bytes.length` bytes in the core's memory via `cabi_realloc` and writes them. */
  _writeBuf(bytes) {
    const realloc = this.core.instance.exports.cabi_realloc;
    const ptr = realloc(0, 0, 1, bytes.length || 1);
    new Uint8Array(this.core.instance.exports.memory.buffer).set(bytes, ptr);
    return ptr;
  }

  init(initArgRecords = []) {
    const bytes = encodeInitArgs(initArgRecords);
    const ptr = this._writeBuf(bytes);
    this.core.instance.exports.init(ptr, bytes.length);
  }

  onEvent(event) {
    const bytes = encodeEvent(event);
    const ptr = this._writeBuf(bytes);
    this.core.instance.exports["on-event"](ptr, bytes.length);
  }
}
