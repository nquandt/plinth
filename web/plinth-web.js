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
  return { manifestText, appWasm };
}

function manifestEntry(manifestText) {
  const m = manifestText.match(/entry\s*=\s*"([^"]+)"/);
  return m ? m[1] : "app.wasm";
}

const DeniedReason = { undeclared: 0, refused: 1, unsupported: 2 };

/**
 * Builds the JS implementations of the core's WIT-level host imports
 * (`plinth:app/{ui,dev,time,store,clipboard}@1.0.0`, SPEC.md §8.5). `onCommit`
 * is called with a decoded op list for every `ui.commit`. `log` defaults to
 * `console.log`.
 *
 * Store and clipboard are stubs: every call returns `denied(unsupported)`,
 * same as a host build with no implementation (SPEC.md §11). Timers are
 * real (JS `setTimeout`/`setInterval`), driven by `scheduleTimerEvent`.
 */
function hostImports(getMemory, onCommit, { log = (s) => console.log(s), scheduleTimerEvent, cancelTimerEvent } = {}) {
  function mem() {
    return new DataView(getMemory().buffer);
  }
  function bytes() {
    return new Uint8Array(getMemory().buffer);
  }
  function readString(ptr, len) {
    return new TextDecoder().decode(bytes().subarray(ptr, ptr + len));
  }
  // result<_, host-error>: tag@0 (1 byte), err payload (host-error: tag@1, reason@2).
  function writeDeniedUnit(retptr) {
    const v = mem();
    v.setUint8(retptr, 1);
    v.setUint8(retptr + 1, 0);
    v.setUint8(retptr + 2, DeniedReason.unsupported);
  }
  // result<T, host-error> where T contains an i32 (option<string>, list<string>, u32):
  // outer align is 4, so tag@0 and the payload (ok or err) start at offset 4.
  function writeDeniedAt4(retptr) {
    const v = mem();
    v.setUint8(retptr, 1);
    v.setUint8(retptr + 4, 0);
    v.setUint8(retptr + 5, DeniedReason.unsupported);
  }

  let nextTimer = 1;
  const timers = new Map(); // id -> JS handle

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
    },
    "plinth:app/store@1.0.0": {
      "kv-get"(_keyPtr, _keyLen, retptr) {
        writeDeniedAt4(retptr);
      },
      "kv-set"(_keyPtr, _keyLen, _valPtr, _valLen, retptr) {
        writeDeniedUnit(retptr);
      },
      "kv-delete"(_keyPtr, _keyLen, retptr) {
        writeDeniedUnit(retptr);
      },
      "kv-keys"(retptr) {
        writeDeniedAt4(retptr);
      },
    },
    "plinth:app/clipboard@1.0.0": {
      "write-text"(_ptr, _len, retptr) {
        writeDeniedUnit(retptr);
      },
      "read-text"(retptr) {
        writeDeniedAt4(retptr);
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
  const imports = hostImports(() => instanceRef.exports.memory, opts.onCommit ?? (() => {}), opts);
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

  /** `corePromiseOrBytes` and `appPromiseOrBytes` are `Uint8Array`s (already fetched). */
  async load(coreBytes, appBytes, opts = {}) {
    this.core = await loadCore(coreBytes, { ...opts, onCommit: (ops) => this.onCommit(ops) });
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
