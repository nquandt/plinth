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

import { openZip, checkPlntProfile } from "./zip.js";
import { decodeOps, encodeEvent, encodeInitArgs } from "./protocol.js";
import { FileSpace, memoryFileBackend, FILES_CAPABILITY } from "./files.js";

const RT_PREFIX = "__plinth_rt_";
/** The `init` record of the `Math.random` seed (core 1.12): 8 bytes, a little-endian u64. */
export const INIT_RANDOM_SEED = 3;

/** The memory cap of an app, the same as the desktop runner (`Limits`, 64 MiB). */
export const MEMORY_LIMIT_PAGES = 1024;

/**
 * The time limit of one call into an app (`init` or one event), the same as
 * the desktop runner (`Limits`, 2 s). In a browser the app runs in a Worker
 * (`app-worker.js`); the proxy ends the Worker at the limit.
 */
export const CALL_TIME_LIMIT_MS = 2000;

/** The stop reason at the time limit: the desktop's words. */
export const TIME_LIMIT_REASON = "the app is not responding (time limit reached)";

/**
 * Returns a copy of a core module whose defined memory has a maximum of
 * `maxPages` (64 KiB pages; GAPS "R1/R2 design"). The core declares no
 * maximum; the desktop caps it in wasmtime instead. With the maximum, a
 * `memory.grow` past the cap fails, and the core's allocator stops the app
 * with "out of memory", the same reason as on the desktop. A lower
 * maximum in the module stays. A module with no memory section is
 * returned unchanged.
 */
export function capCoreMemory(bytes, maxPages) {
  const u8s = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let pos = 8;
  const leb = () => {
    let result = 0,
      shift = 0,
      byte;
    do {
      byte = u8s[pos++];
      result += (byte & 0x7f) * 2 ** shift;
      shift += 7;
    } while (byte & 0x80);
    return result;
  };
  const enc = (n) => {
    const out = [];
    do {
      let b = n % 128;
      n = Math.floor(n / 128);
      if (n > 0) b |= 0x80;
      out.push(b);
    } while (n > 0);
    return out;
  };
  while (pos < u8s.length) {
    const start = pos;
    const id = u8s[pos++];
    const len = leb();
    const body = pos;
    const end = body + len;
    if (id === 5) {
      const count = leb();
      const out = [...enc(count)];
      for (let i = 0; i < count; i++) {
        const flags = u8s[pos++];
        const min = leb();
        const max = flags & 1 ? leb() : Infinity;
        out.push(flags | 1, ...enc(min), ...enc(Math.max(min, Math.min(max, maxPages))));
      }
      if (pos !== end) return u8s; // an encoding this function does not know: leave the module as it is
      const section = [5, ...enc(out.length), ...out];
      const result = new Uint8Array(start + section.length + (u8s.length - end));
      result.set(u8s.subarray(0, start), 0);
      result.set(section, start);
      result.set(u8s.subarray(end), start + section.length);
      return result;
    }
    pos = end;
  }
  return u8s;
}

/** The custom section of an app module with its source map (docs/PRODUCTION.md L9). */
export const SRCMAP_SECTION = "plinth-srcmap";

/**
 * The function bodies of a raw wasm module, by function index (imports
 * included): `[start, end)` of each body, after the body size. Also the
 * content of the custom section `customName`, if the module has it.
 * `null` for bytes that are not a valid module.
 */
export function moduleBodies(bytes, customName) {
  try {
    const u8s = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    let pos = 8;
    const u8 = () => {
      if (pos >= u8s.length) throw new Error("the module ends early");
      return u8s[pos++];
    };
    const leb = () => {
      let result = 0,
        shift = 0,
        byte;
      do {
        byte = u8();
        result += (byte & 0x7f) * 2 ** shift;
        shift += 7;
      } while (byte & 0x80);
      return result;
    };
    const name = () => {
      const len = leb();
      pos += len;
      return new TextDecoder().decode(u8s.subarray(pos - len, pos));
    };
    const out = { bodies: [], custom: null };
    while (pos < u8s.length) {
      const id = u8();
      const len = leb();
      const end = pos + len;
      if (end > u8s.length) return null;
      if (id === 0 && customName !== undefined && name() === customName) {
        out.custom = u8s.subarray(pos, end);
      } else if (id === 2) {
        for (let n = leb(); n > 0; n--) {
          name();
          name();
          const kind = u8();
          if (kind === 0) {
            leb();
            out.bodies.push(null); // an imported function has no body
          } else if (kind === 1 || kind === 2) {
            if (kind === 1) u8(); // element type
            const flags = u8();
            leb();
            if (flags & 1) leb();
          } else if (kind === 3) {
            u8();
            u8();
          } else return null;
        }
      } else if (id === 10) {
        for (let n = leb(); n > 0; n--) {
          const size = leb();
          out.bodies.push([pos, pos + size]);
          pos += size;
        }
      }
      pos = end;
    }
    return out;
  } catch {
    return null;
  }
}

/**
 * Reads a `plinth-srcmap` section (the format is in
 * `crates/plinth-link/src/srcmap.rs`): `{ files, funcs }`, where `funcs`
 * maps a function index to `{ file, entries: [[offset, line], ...] }`.
 * `null` for bytes that are not valid: a bad map never stops an app.
 */
export function decodeSrcMap(data) {
  try {
    let pos = 0;
    const u8 = () => {
      if (pos >= data.length) throw new Error("the source map ends early");
      return data[pos++];
    };
    const leb = () => {
      let result = 0,
        shift = 0,
        byte;
      do {
        byte = u8();
        result += (byte & 0x7f) * 2 ** shift;
        shift += 7;
      } while (byte & 0x80);
      return result;
    };
    const sleb = () => {
      let result = 0,
        shift = 0,
        byte;
      do {
        byte = u8();
        result += (byte & 0x7f) * 2 ** shift;
        shift += 7;
      } while (byte & 0x80);
      return byte & 0x40 ? result - 2 ** shift : result;
    };
    if (u8() !== 1) return null;
    const files = [];
    for (let n = leb(); n > 0; n--) {
      const len = leb();
      if (pos + len > data.length) return null;
      files.push(new TextDecoder().decode(data.subarray(pos, pos + len)));
      pos += len;
    }
    const funcs = new Map();
    let next = 0;
    for (let n = leb(); n > 0; n--) {
      const func = next + leb();
      next = func + 1;
      const file = files.length > 1 ? leb() : 0;
      if (file >= files.length) return null;
      const entries = [];
      let off = 0,
        line = 0;
      for (let k = leb(); k > 0; k--) {
        off += leb();
        line += sleb();
        entries.push([off, line]);
      }
      funcs.set(func, { file, entries });
    }
    return pos === data.length ? { files, funcs } : null;
  } catch {
    return null;
  }
}

/**
 * The source place (`app/main.tsx:12`) of the innermost app frame in the
 * stack of a trap, or `null`. The engine writes a wasm frame as
 * `wasm-function[N]:0xOFFSET` (OFFSET in the module). `app` and `core` are
 * the results of `moduleBodies` for the app and the core module: a frame
 * that fits a core body is a core frame, not an app frame.
 */
export function trapPlace(stack, app, core) {
  if (!app?.custom || typeof stack !== "string") return null;
  const map = decodeSrcMap(app.custom);
  if (!map) return null;
  const fits = (m, n, off) => {
    const b = m?.bodies[n];
    return b ? off >= b[0] && off < b[1] : false;
  };
  for (const [, n, hex] of stack.matchAll(/wasm-function\[(\d+)\]:0x([0-9a-f]+)/gi)) {
    const func = Number(n),
      off = parseInt(hex, 16);
    if (!fits(app, func, off) || fits(core, func, off)) continue;
    const f = map.funcs.get(func);
    if (!f) continue;
    const rel = off - app.bodies[func][0];
    let line = null;
    for (const [o, l] of f.entries) {
      if (o > rel) break;
      line = l;
    }
    if (line !== null) return `${map.files[f.file]}:${line}`;
  }
  return null;
}

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
  await checkPlntProfile(zip);
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

/**
 * A `plinth:notify` backend on the browser's `Notification` API (SPEC.md
 * §11, docs/host-apis.md "plinth:notify"): `{ show(title, body, tag),
 * cancel(tag) }`. `show` resolves with "shown", "unsupported" or
 * "denied:refused" (the user or the browser blocked notifications for this
 * page). It asks for the permission when the browser has not asked yet
 * (`Notification.requestPermission()`); a browser can refuse to ask
 * without a user gesture, and then the answer is "unsupported". A page
 * with an opaque origin (a sandboxed frame) cannot use the API: it answers
 * "unsupported". `tag` is the `tag` option, so the browser replaces an
 * earlier notification with the same tag; `cancel(tag)` closes the open
 * notification with that tag. `opts`: `icon` (a URL), `origin` (default
 * the origin of this page), `log` (default `console.warn`).
 */
export function browserNotify(N = globalThis.Notification, opts = {}) {
  const log = opts.log ?? ((s) => console.warn(s));
  const origin = opts.origin ?? globalThis.origin;
  const usable = typeof N === "function" && origin !== "null";
  const open = new Map(); // tag -> the open Notification
  return {
    async show(title, body, tag) {
      if (!usable) return "unsupported";
      let permission = N.permission;
      if (permission === "default" && typeof N.requestPermission === "function") {
        try {
          permission = await N.requestPermission();
        } catch (e) {
          log(`[plinth] notify: the browser did not ask for the permission: ${e?.message ?? e}`);
          return "unsupported";
        }
      }
      if (permission === "denied") return "denied:refused";
      if (permission !== "granted") {
        // The user closed the question, or the browser did not ask (no user gesture).
        log(`[plinth] notify: no permission for notifications (${permission})`);
        return "unsupported";
      }
      const options = {};
      if (body !== null && body !== undefined) options.body = body;
      if (tag !== null && tag !== undefined) options.tag = tag;
      if (opts.icon) options.icon = opts.icon;
      let n;
      try {
        n = new N(title, options);
      } catch (e) {
        // For example Chrome on Android: only a service worker can show one.
        log(`[plinth] notify: the browser refused the notification: ${e?.message ?? e}`);
        return "unsupported";
      }
      if (tag !== null && tag !== undefined) {
        open.set(tag, n);
        n.onclose = () => {
          if (open.get(tag) === n) open.delete(tag);
        };
      }
      return "shown";
    },
    cancel(tag) {
      const n = open.get(tag);
      if (!n) return;
      open.delete(tag);
      n.close();
    },
  };
}

/**
 * True for a host that names the local network rather than the public
 * internet (`net.local`, SPEC.md §11): `localhost` and `*.localhost`, a
 * `.local` name, or an IP literal in a non-public range: unspecified,
 * loopback, private, shared (CGNAT), link-local (this includes the cloud
 * metadata address 169.254.169.254), multicast, broadcast and reserved
 * ranges, IPv6 unique-local and site-local addresses, and an IPv4 address
 * embedded in IPv6 (mapped, compatible, NAT64, 6to4). The same rules as
 * `is_private_host` in `crates/plinth-runner-wasmtime/src/policy.rs`.
 * `new URL()` normalizes other IPv4 forms (`2130706433`, `0x7f.1`) to
 * dotted decimal first. A page cannot check where a public name
 * resolves; the desktop host can (net.rs).
 */
export function isPrivateNetHost(host) {
  let h = host.toLowerCase();
  if (h.startsWith("[")) h = h.slice(1, h.indexOf("]") < 0 ? h.length : h.indexOf("]"));
  else if (h.split(":").length === 2) h = h.split(":")[0]; // strip a port
  if (h.endsWith(".")) h = h.slice(0, -1);
  if (h === "localhost" || h.endsWith(".localhost") || h.endsWith(".local")) return true;
  const v4 = parseIPv4(h);
  if (v4) return isPrivateV4(v4);
  const v6 = parseIPv6(h);
  return v6 ? isPrivateV6(v6) : false;
}

function parseIPv4(h) {
  const m = h.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!m) return null;
  const o = m.slice(1).map(Number);
  return o.every((n) => n <= 255) ? o : null;
}

/** Eight 16-bit segments, or null. Accepts `::` and a dotted IPv4 tail. */
function parseIPv6(h) {
  if (!h.includes(":")) return null;
  const lastColon = h.lastIndexOf(":");
  const v4 = parseIPv4(h.slice(lastColon + 1));
  if (v4) {
    const hex = (hi, lo) => ((hi << 8) | lo).toString(16);
    h = `${h.slice(0, lastColon + 1)}${hex(v4[0], v4[1])}:${hex(v4[2], v4[3])}`;
  }
  const halves = h.split("::");
  if (halves.length > 2) return null;
  const part = (s) => (s === "" ? [] : s.split(":").map((g) => (/^[0-9a-f]{1,4}$/.test(g) ? parseInt(g, 16) : NaN)));
  const head = part(halves[0]);
  const rest = halves.length === 2 ? part(halves[1]) : [];
  const count = head.length + rest.length;
  if ([...head, ...rest].some(Number.isNaN)) return null;
  if (halves.length === 1 ? count !== 8 : count > 7) return null;
  return [...head, ...new Array(8 - count).fill(0), ...rest];
}

function isPrivateV4([a, b, c]) {
  return (
    a === 0 || a === 127 || a === 10 ||
    (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) ||
    (a === 169 && b === 254) || (a === 100 && b >= 64 && b <= 127) ||
    (a === 192 && b === 0 && c === 0) || (a === 198 && (b === 18 || b === 19)) ||
    a >= 224 // multicast, reserved and broadcast
  );
}

function isPrivateV6(s) {
  const embedded = (hi, lo) => isPrivateV4([hi >> 8, hi & 255, lo >> 8, lo & 255]);
  const zeroTo = (n) => s.slice(0, n).every((x) => x === 0);
  if (zeroTo(5) && s[5] === 0xffff) return embedded(s[6], s[7]); // ::ffff:a.b.c.d
  if (zeroTo(6)) return (s[6] === 0 && s[7] <= 1) || embedded(s[6], s[7]); // ::, ::1, ::a.b.c.d
  if (s[0] === 0x64 && s[1] === 0xff9b && s.slice(2, 6).every((x) => x === 0)) return embedded(s[6], s[7]);
  if (s[0] === 0x2002) return embedded(s[1], s[2]);
  return (s[0] & 0xff00) === 0xff00 || (s[0] & 0xfe00) === 0xfc00 || (s[0] & 0xffc0) === 0xfe80 || (s[0] & 0xffc0) === 0xfec0;
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
 * Frame timers (core 1.12, `onFrame`) share one `requestFrame` loop (default
 * `requestAnimationFrame`, which the browser stops in a hidden page; the
 * loop stops with `cancelFrame` when the last frame timer stops) and fire
 * through `scheduleFrameEvent(id, dt)`.
 */
/**
 * The default dialog answers: the browser's own dialogs when they exist
 * (`alert` → null, `confirm` → boolean, `prompt` → string or null), else
 * (Node) a declined answer.
 */
export function defaultAskDialog(kind, message) {
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
    scheduleFrameEvent,
    requestFrame = globalThis.requestAnimationFrame?.bind(globalThis) ?? ((cb) => setTimeout(() => cb(performance.now()), 16)),
    cancelFrame = globalThis.cancelAnimationFrame?.bind(globalThis) ?? clearTimeout,
    kvStore,
    capabilities = new Set(),
    refused = new Set(),
    askDialog,
    completeRequest,
    clipboard = null,
    hub = null,
    netFetch = null,
    files = null,
    onCommitBytes = null,
    notify = null,
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
  // plinth:notify (core 1.12, docs/PRODUCTION.md H3): the same request id
  // and `[ok, 0, text, error]` completion as `net.fetch`. The capability
  // check comes first; a denied call answers "denied:<reason>" and never
  // traps. `notify` (`browserNotify`, or the frame's bridge to the page)
  // answers "shown", "unsupported" or "denied:refused"; with no `notify`
  // (Node), an allowed call answers "unsupported".
  function openNotify(title, body, tag) {
    const id = nextRequest++;
    const reason = capReason("notify");
    const run =
      reason !== null
        ? Promise.resolve([false, 0, "", deniedText(reason)])
        : Promise.resolve()
            .then(() => (notify ? notify.show(title, body, tag) : "unsupported"))
            .catch(() => "unsupported")
            .then((text) =>
              typeof text === "string" && text.startsWith("denied:") ? [false, 0, "", text] : [true, 0, text === "shown" ? "shown" : "unsupported", null],
            );
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
  // Frame timers (core 1.12): one loop for all of them, so they get the same
  // `dt`, the milliseconds since the previous frame (0 for the first).
  const frameTimers = new Set();
  let frameHandle = null; // the pending frame request, while the loop runs
  let lastFrame = null;
  function frameTick(t) {
    frameHandle = null;
    const dt = lastFrame === null ? 0 : Math.max(0, t - lastFrame);
    lastFrame = t;
    for (const id of [...frameTimers]) if (frameTimers.has(id)) scheduleFrameEvent(id, dt);
    if (frameTimers.size > 0 && frameHandle === null) frameHandle = requestFrame(frameTick);
    else if (frameTimers.size === 0) lastFrame = null;
  }
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
        // `onCommitBytes` (the Worker, `app-worker.js`): the raw op bytes,
        // decoded by the document.
        if (onCommitBytes) onCommitBytes(copy);
        else onCommit(decodeOps(copy));
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
      "set-frame-timer"(retptr) {
        if (!scheduleFrameEvent) {
          writeDeniedAt4(retptr);
          return;
        }
        const id = nextTimer++;
        frameTimers.add(id);
        if (frameHandle === null) frameHandle = requestFrame(frameTick);
        const v = mem();
        v.setUint8(retptr, 0); // ok
        v.setUint32(retptr + 4, id, true);
      },
      "cancel-timer"(id) {
        if (frameTimers.delete(id) && frameTimers.size === 0) {
          if (frameHandle !== null) cancelFrame(frameHandle);
          frameHandle = null;
          lastFrame = null;
        }
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
    // Lowered core ABI: title(ptr,len), body(tag,ptr,len), tag(tag,ptr,len)
    // (each `option<string>` as for `net.fetch`'s body). `show` returns the
    // request id directly; `cancel` writes a `result<_, host-error>`.
    "plinth:app/notify@1.0.0": {
      show(titlePtr, titleLen, bodyTag, bodyPtr, bodyLen, tagTag, tagPtr, tagLen) {
        const title = readString(titlePtr, titleLen);
        const body = bodyTag === 0 ? null : readString(bodyPtr, bodyLen);
        const tag = tagTag === 0 ? null : readString(tagPtr, tagLen);
        return openNotify(title, body, tag);
      },
      cancel(ptr, len, retptr) {
        const reason = capReason("notify");
        if (reason !== null) return writeDeniedUnit(retptr, reason);
        const tag = readString(ptr, len);
        // Fire-and-forget: an unknown tag is not an error.
        if (notify) Promise.resolve().then(() => notify.cancel(tag)).catch(() => {});
        writeOkUnit(retptr);
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
  // `opts.memoryLimitPages`: 0 means no cap (tests only).
  const limit = opts.memoryLimitPages ?? MEMORY_LIMIT_PAGES;
  const bytes = limit && !(coreWasmBytes instanceof WebAssembly.Module) ? capCoreMemory(coreWasmBytes, limit) : coreWasmBytes;
  const { instance } = await WebAssembly.instantiate(bytes, imports);
  instanceRef = instance;
  const tableExportName = Object.keys(instance.exports).find(
    (k) => instance.exports[k] instanceof WebAssembly.Table,
  );
  if (!tableExportName) throw new Error("the core does not export its function table (needs --export-table)");
  // The bytes that run, for `trapPlace` (a frame in a core body is not an app frame).
  return { instance, tableExportName, bytes: bytes instanceof WebAssembly.Module ? null : bytes };
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
    /**
     * The plain reason after a trap (the time or memory limit, a failed
     * runtime check), else `null`. A stopped app is never called again, as
     * on the desktop (`plinth-runner-wasmtime` `Guest::call`).
     */
    this.stopped = null;
    /**
     * `(reason, error)` after the app stopped; the DOM renderer shows "This
     * app stopped". With no handler, `init`/`onEvent` throw the error.
     */
    this.onStop = null;
    /** `(message)` for each uncaught app error (core 1.10); the app keeps running. */
    this.onError = null;
    /**
     * `()` before and after each call into the app (`init` or one event,
     * also timers, frames and answers). The Worker (`app-worker.js`) tells
     * the watchdog in the document with them.
     */
    this.onCallStart = null;
    this.onCallEnd = null;
    this._lastTrap = null;
    /** The raw app module, for the place of a trap (`trapPlace`); null for a compiled module. */
    this._appBytes = null;
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
    const scheduleFrameEvent = opts.scheduleFrameEvent ?? ((id, dt) => this.onEvent({ kind: "frame", timer: id, dt }));
    const completeRequest = (id, result) => this.onEvent({ kind: "completion", request: id, result });
    const { id: appId, capabilities } = opts.manifestText ? parseManifest(opts.manifestText) : { id: "", capabilities: new Set() };
    const kvStore =
      opts.kvStore ?? (opts.storage ? localStorageKvStore(opts.storage, `plinth:${appId}:`) : mapKvStore(opts.sharedMap));
    const log = opts.log ?? ((s) => console.log(s));
    const reportError = opts.reportError ?? ((s) => console.error(s));
    this.core = await loadCore(coreBytes, {
      ...opts,
      // The core logs `trap: <reason>` before it traps (plinth_rt::trap).
      log: (s) => {
        if (typeof s === "string" && s.startsWith("trap: ")) this._lastTrap = s.slice(6);
        log(s);
      },
      reportError: (s) => {
        reportError(s);
        this.onError?.(s);
      },
      scheduleTimerEvent,
      scheduleFrameEvent,
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
    this._appBytes = appBytes instanceof WebAssembly.Module ? null : appBytes;
    return this;
  }

  /** Allocates `bytes.length` bytes in the core's memory via `cabi_realloc` and writes them. */
  _writeBuf(bytes) {
    const realloc = this.core.instance.exports.cabi_realloc;
    const ptr = realloc(0, 0, 1, bytes.length || 1);
    new Uint8Array(this.core.instance.exports.memory.buffer).set(bytes, ptr);
    return ptr;
  }

  /**
   * Calls `init` with the init records. Without a `RANDOM_SEED` record (tag
   * 3), it adds one with fresh entropy (core 1.12, `Math.random`); a test
   * gives its own seed to repeat a run.
   */
  init(initArgRecords = []) {
    const records = [...initArgRecords];
    if (!records.some((r) => r.tag === INIT_RANDOM_SEED)) records.push({ tag: INIT_RANDOM_SEED, data: crypto.getRandomValues(new Uint8Array(8)) });
    const bytes = encodeInitArgs(records);
    this._call(() => {
      const ptr = this._writeBuf(bytes);
      this.core.instance.exports.init(ptr, bytes.length);
    });
  }

  onEvent(event) {
    // A timer, a frame or an answer can come after the app stopped.
    if (this.stopped !== null) return;
    const bytes = encodeEvent(event);
    this._call(() => {
      const ptr = this._writeBuf(bytes);
      this.core.instance.exports["on-event"](ptr, bytes.length);
    });
  }

  _call(f) {
    if (this.stopped !== null) return;
    this._lastTrap = null;
    this.onCallStart?.();
    // More stack frames (V8) than the default 10, so that a trap deep in
    // the core still shows the innermost app frame (`trapPlace`).
    const stackLimit = Error.stackTraceLimit;
    Error.stackTraceLimit = 64;
    try {
      f();
    } catch (err) {
      Error.stackTraceLimit = stackLimit;
      this.stopped = stopReason(err, this._lastTrap, this._trapPlace(err));
      if (!this.onStop) throw err;
      console.error(`[plinth] the app stopped: ${this.stopped}`, err);
      this.onStop(this.stopped, err);
    } finally {
      Error.stackTraceLimit = stackLimit;
      this.onCallEnd?.();
    }
  }

  /** The source place of a trap from the app's source map, or null (no map: an old package). */
  _trapPlace(err) {
    if (!this._appBytes) return null;
    const app = moduleBodies(this._appBytes, SRCMAP_SECTION);
    if (!app?.custom) return null;
    return trapPlace(String(err?.stack ?? ""), app, this.core.bytes ? moduleBodies(this.core.bytes) : null);
  }
}

/**
 * A short, plain reason for a trap, the same words as the desktop host
 * (`stop_reason` in `plinth-runner-wasmtime`). With `place` (from
 * `trapPlace`), the reason ends with it: "array index out of bounds
 * (app/main.tsx:12)".
 */
export function stopReason(err, lastTrap, place = null) {
  const reason = plainReason(err, lastTrap);
  return place ? `${reason} (${place})` : reason;
}

function plainReason(err, lastTrap) {
  if (lastTrap) return lastTrap;
  const msg = String(err?.message ?? err);
  if (err instanceof RangeError && /call stack/i.test(msg)) return "too many nested calls (stack overflow)";
  if (typeof WebAssembly !== "undefined" && err instanceof WebAssembly.RuntimeError) {
    if (/unreachable/i.test(msg)) return "a check in the app failed";
    if (/divide by zero|division by zero/i.test(msg)) return "an integer division by zero";
    if (/out of bounds/i.test(msg)) return "an invalid memory access";
  }
  return msg;
}
