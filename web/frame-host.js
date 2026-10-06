// The parent side of a sandboxed Plinth app (docs/web-hub.md §4,
// web/README.md "The app-frame bridge"). Every Plinth app on the web runs
// in `<iframe sandbox="allow-scripts" src="app-frame.html">`: an opaque
// origin, so it cannot read the storage, the cookies or the DOM of the page
// or of a different app. `app-frame.js` is the frame side (the runtime,
// the core, the app and the DOM renderer); this module is the parent side,
// for any page that shows an app (the web App Hub, and later a self-hosted
// export):
//
// - storage: the kv data of each app, in its own namespace of the page's
//   storage (`hub-storage.js`), sent to the frame as a snapshot at start;
// - capability checks: the frame checks each call, and this side checks
//   again (the frame runs untrusted app code);
// - `fetch` for `plinth:net`, made by the page (so a request carries the
//   page's real origin, not "null"), after the same `net:` check;
// - the clipboard, through the page;
// - dialogs (`plinth:dialog`), drawn by the page;
// - the content height that the frame reports (`autoHeight`).
//
// The page decides what the app may do (consent, grants) and gives the
// result as `declared` and `refused`.

import { checkFrameMessage, applyKvMessage, startMessage, CHANNEL, KV_QUOTA } from "./hub-storage.js";
import { netDenied, httpFetch } from "./plinth-web.js";

/** `el("p", { class: "x" }, "text", child)`: a small DOM builder (text is never parsed as HTML). */
function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === "class") node.className = v;
    else node.setAttribute(k, v === true ? "" : String(v));
  }
  for (const c of children.flat()) {
    if (c === null || c === undefined || c === false) continue;
    node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return node;
}

/**
 * A dialog that the page draws for `plinth:dialog` (a sandboxed frame
 * without `allow-modals` cannot show one). `alert` answers null, `confirm`
 * true or false, `prompt` the text or null. `title` names the app.
 */
export function hostDialog(kind, message, title = "") {
  return new Promise((resolve) => {
    const input = kind === "prompt" ? el("input", { class: "host-input", type: "text", "aria-label": message }) : null;
    const buttons = [el("button", { class: "host-button host-primary", value: "ok" }, "OK")];
    if (kind !== "alert") buttons.push(el("button", { class: "host-button", value: "cancel" }, "Cancel"));
    const dialog = el(
      "dialog",
      { class: "host-dialog", "aria-labelledby": "host-app-dialog-title", "aria-describedby": "host-app-dialog-text" },
      el("h2", { id: "host-app-dialog-title" }, title || "Message"),
      el("p", { id: "host-app-dialog-text" }, message),
      el("form", { method: "dialog" }, input, el("div", { class: "host-actions" }, buttons)),
    );
    dialog.addEventListener("close", () => {
      const ok = dialog.returnValue === "ok";
      dialog.remove();
      if (kind === "alert") resolve(null);
      else if (kind === "confirm") resolve(ok);
      else resolve(ok ? input.value : null);
    });
    document.body.append(dialog);
    dialog.showModal();
  });
}

/**
 * One app in a sandboxed frame.
 *
 * Options:
 * - `appId`, `title`: the app (the kv namespace, the frame title).
 * - `pkg`, `core`: the checked `.plnt` and core bytes (`Uint8Array`).
 * - `declared`: the capability names of the manifest; `refused`: the ones
 *   that the user did not allow. A refused call answers "denied" in the app.
 * - `store`: a `HubStore` (kv data of each app); `quota`: characters.
 * - `src`: the frame page (default `app-frame.html`); or `srcdoc`: the
 *   HTML of the frame page (a web export, SPEC.md §10.3).
 * - `autoHeight`: size the iframe to the content height that the frame reports.
 * - `askDialog(kind, message)`: default `hostDialog`.
 * - `fetchImpl`: default `fetch`.
 * - `onStarted()`, `onFailed(message)`.
 *
 * `element` is the iframe; put it in the page. `destroy()` removes it.
 */
export class AppFrame {
  constructor(opts) {
    this.opts = { src: "app-frame.html", quota: KV_QUOTA, autoHeight: false, fetchImpl: globalThis.fetch, ...opts };
    this.declared = new Set(opts.declared ?? []);
    this.refused = new Set(opts.refused ?? []);
    this.sent = false;
    // `allow-scripts` only: an opaque origin, no forms, no popups, no modal
    // dialogs, no top navigation, no same-origin access.
    this.element = el("iframe", { class: "host-frame", sandbox: "allow-scripts", title: opts.title ?? opts.appId });
    if (opts.srcdoc !== undefined) this.element.srcdoc = opts.srcdoc;
    else this.element.src = this.opts.src;
    this.onMessage = (event) => this.handle(event);
    window.addEventListener("message", this.onMessage);
  }

  destroy() {
    window.removeEventListener("message", this.onMessage);
    this.element.remove();
  }

  allowed(name) {
    return this.declared.has(name) && !this.refused.has(name);
  }

  answer(id, value) {
    // The frame has an opaque origin: "*" is the only target origin that
    // reaches it. The message goes to the window of this frame only.
    this.element.contentWindow?.postMessage({ channel: CHANNEL, type: "answer", id, value }, "*");
  }

  handle(event) {
    if (event.source !== this.element.contentWindow) return;
    const msg = checkFrameMessage(event.data);
    if (!msg) return;
    this.element.dataset.last = msg.type; // the last message, for tests and debugging
    const { appId, store } = this.opts;
    if (msg.type === "kv-set" || msg.type === "kv-delete") {
      if (this.allowed("store.kv")) applyKvMessage(store, appId, msg, this.opts.quota);
      return;
    }
    switch (msg.type) {
      case "ready": {
        if (this.sent) return;
        this.sent = true;
        const pkg = this.opts.pkg.slice().buffer;
        const core = this.opts.core.slice().buffer;
        const start = startMessage(store, appId, { pkg, core, refused: this.refused, quota: this.opts.quota });
        if (!this.allowed("store.kv")) start.kv = {};
        this.element.contentWindow.postMessage(start, "*", [pkg, core]);
        break;
      }
      case "started":
        this.opts.onStarted?.();
        break;
      case "failed":
        this.opts.onFailed?.(msg.message);
        break;
      case "size":
        if (this.opts.autoHeight) this.element.style.height = `${Math.min(msg.height, 100000)}px`;
        break;
      case "clipboard-write":
        if (this.allowed("clipboard.write")) navigator.clipboard?.writeText(msg.text).catch(() => {});
        break;
      case "clipboard-read":
        if (!this.allowed("clipboard.read") || !navigator.clipboard?.readText) this.answer(msg.id, null);
        else
          navigator.clipboard.readText().then(
            (text) => this.answer(msg.id, text),
            () => this.answer(msg.id, null),
          );
        break;
      case "net-fetch": {
        const denied = netDenied(msg.url, this.declared, this.refused);
        if (denied) this.answer(msg.id, [false, 0, "", denied]);
        else httpFetch(msg.url, msg.method, msg.headers, msg.body, this.opts.fetchImpl).then((r) => this.answer(msg.id, r));
        break;
      }
      case "dialog": {
        const ask = this.opts.askDialog ?? ((kind, message) => hostDialog(kind, message, this.opts.title));
        Promise.resolve()
          .then(() => ask(msg.kind, msg.message))
          .then(
            (value) => this.answer(msg.id, value ?? null),
            () => this.answer(msg.id, null),
          );
        break;
      }
    }
  }
}
