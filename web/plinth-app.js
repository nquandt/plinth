// The `<plinth-app>` element of a web export (SPEC.md §10.3). A page loads
// `plinth.js` as a module script and then holds, for example:
//
//   <plinth-app src="todo.plnt" core="plinth-core-1.10.wasm"></plinth-app>
//
// The element reads the package, finds the core, and runs the app in a
// sandboxed iframe (`AppFrame`, an opaque origin). The frame document is
// inline (`srcdoc`): it holds the whole web host as one script, so the
// export needs no server logic, no CORS headers and no second page, and a
// single-file export also works from disk.
//
// A web export works like a native export: no Hub and no consent window.
// The host gives the app the capabilities that its manifest declares. The
// kv data of the app belongs to the origin of the page.
//
// This file is only a part of the bundle `plinth.js`
// (`scripts/gen-plinth-js.mjs`); the bundle gives `defineElement` the frame
// document and its own URL.

import { readPlnt, parseManifest } from "./plinth-web.js";
import { AppFrame } from "./frame-host.js";
import { HubStore, memoryStorage } from "./hub-storage.js";

/** The bytes of a base64 text (white space is ignored). */
export function base64Bytes(text) {
  const bin = atob(String(text).replace(/\s+/g, ""));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/**
 * The bytes that `ref` names: `#id` is an inline element (its text is
 * base64), any other value is a URL relative to the page.
 */
export async function loadBytes(ref, doc = document) {
  if (ref.startsWith("#")) {
    const node = doc.getElementById(ref.slice(1));
    if (!node) throw new Error(`no element with the id "${ref.slice(1)}"`);
    return base64Bytes(node.textContent);
  }
  const url = new URL(ref, doc.baseURI);
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url.href}: HTTP ${r.status}`);
  return new Uint8Array(await r.arrayBuffer());
}

/** The default core file of an app: `plinth-core-<version>.wasm` next to `plinth.js`. */
export function defaultCoreUrl(runtime, baseUrl) {
  const version = String(runtime ?? "").replace(/^plinth-core\//, "") || "1.0";
  return new URL(`plinth-core-${version}.wasm`, baseUrl).href;
}

/** `localStorage`, or a memory store if the page cannot use it (some file:// pages, private modes). */
function pageStorage(name) {
  try {
    const s = globalThis[name];
    s.getItem("plinth-probe");
    return s;
  } catch {
    return memoryStorage();
  }
}

/**
 * The style of the page side: the element, and the dialogs that the page
 * draws for the app. The colors are the tokens of `style.css`, on the
 * dialog only, so they do not change the page.
 */
function pageCss(styleCss) {
  const [dark = "", light = ""] = (styleCss.match(/:root\s*\{[^}]*\}/g) ?? []).map((b) => b.replace(/^:root/, ".host-dialog"));
  return `
plinth-app { display: block; }
plinth-app[height="fill"] { height: 100%; }
plinth-app > .host-frame { display: block; width: 100%; border: 0; background: transparent; }
plinth-app[height="fill"] > .host-frame { height: 100%; }
plinth-app > .plinth-app-error { margin: 0; padding: 16px; white-space: pre-wrap; color: #c62828; font-family: ui-monospace, Consolas, monospace; }
${dark}
@media (prefers-color-scheme: light) { ${light} }
.host-dialog { background: var(--pl-bg); color: var(--pl-text); border: 1px solid var(--pl-border); border-radius: 12px;
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif; line-height: 1.45;
  width: min(480px, calc(100vw - 32px)); box-sizing: border-box; padding: 16px 20px; }
.host-dialog::backdrop { background: rgb(0 0 0 / 0.55); }
.host-dialog h2 { font-size: 1.2rem; margin: 0 0 8px; }
.host-actions { display: flex; flex-wrap: wrap; gap: 8px 12px; margin: 16px 0 0; }
.host-button { font: inherit; border-radius: 8px; padding: 8px 16px; cursor: pointer;
  background: var(--pl-surface); color: var(--pl-text); border: 1px solid var(--pl-border); }
.host-primary { background: var(--pl-accent); color: var(--pl-accent-text); border-color: var(--pl-accent); font-weight: 600; }
.host-input { width: 100%; box-sizing: border-box; font: inherit; padding: 8px 10px; border-radius: 6px;
  border: 1px solid var(--pl-border); background: var(--pl-surface); color: var(--pl-text); }
`;
}

/**
 * Defines `<plinth-app>` (one time).
 *
 * - `frameDocument({ title, fill })`: the HTML of the app frame.
 * - `baseUrl`: the URL of `plinth.js`, for the default core file.
 * - `styleCss`: the text of `style.css` (for the dialog tokens).
 *
 * Attributes: `src` (a URL or `#id` of a `.plnt`), `core` (a URL or `#id`
 * of the core; default `defaultCoreUrl`), `height` (`content`, the
 * default: the element takes the height of the app; `fill`: the app fills
 * the height that the page gives the element).
 *
 * The element sets `data-state` to `loading`, `running` or `failed`.
 */
export function defineElement({ frameDocument, baseUrl, styleCss }) {
  if (typeof customElements === "undefined" || customElements.get("plinth-app")) return;
  const style = document.createElement("style");
  style.textContent = pageCss(styleCss);
  document.head.append(style);

  class PlinthAppElement extends HTMLElement {
    connectedCallback() {
      if (this.frame || this.loading) return;
      this.loading = this.start()
        .catch((err) => this.fail(err))
        .finally(() => (this.loading = null));
    }

    disconnectedCallback() {
      this.frame?.destroy();
      this.frame = null;
    }

    fail(err) {
      console.error(err);
      this.dataset.state = "failed";
      const pre = document.createElement("pre");
      pre.className = "plinth-app-error";
      pre.setAttribute("role", "alert");
      pre.textContent = `This app did not start: ${err?.message ?? err}`;
      this.replaceChildren(pre);
    }

    async start() {
      this.dataset.state = "loading";
      const src = this.getAttribute("src");
      if (!src) throw new Error("<plinth-app> needs a src attribute");
      const pkg = await loadBytes(src);
      const { manifestText } = await readPlnt(pkg);
      const manifest = parseManifest(manifestText);
      const core = await loadBytes(this.getAttribute("core") ?? defaultCoreUrl(manifest.runtime, baseUrl));
      if (!this.isConnected) return;
      const title = manifest.name || manifest.id || "Plinth app";
      const fill = this.getAttribute("height") === "fill";
      const frame = new AppFrame({
        appId: manifest.id,
        title,
        pkg,
        core,
        declared: [...manifest.capabilities],
        refused: [],
        store: new HubStore(pageStorage("localStorage"), pageStorage("sessionStorage")),
        srcdoc: frameDocument({ title, fill }),
        autoHeight: !fill,
        onStarted: () => (this.dataset.state = "running"),
        onFailed: (message) => this.fail(new Error(message)),
      });
      this.frame = frame;
      this.replaceChildren(frame.element);
    }
  }
  customElements.define("plinth-app", PlinthAppElement);
}
