// A minimal DOM renderer for the op buffer (SPEC.md §8.2), for the web
// host spike. Applies ops to a semantic tree of plain objects, then
// renders Screen/Section/Text/Heading/Button/TextField/Toggle/List/Row/
// Group/Empty (plus a nav shell for primary screens) as plain semantic
// HTML, and sends `ui` events (press/change) back through `on-event`.
//
// This file is DOM-only: it does not touch wasm directly. `app.onCommit`
// (from plinth-web.js) feeds it ops; it calls `app.onEvent(...)` back.

import { ControlKind, Prop, Event, EnumAspect, EnumTone, EnumDatePickerMode, EnumChartKind } from "./ui-api.js";

const ASPECT_RATIO = { [EnumAspect.square]: "1 / 1", [EnumAspect.wide]: "16 / 9", [EnumAspect.tall]: "3 / 4" };

/** Icon name -> glyph, matching `plinth-ui`'s `theme::icon_glyph` (SPEC.md §6.3). */
const ICON_GLYPH = {
  house: "⌂", home: "⌂",
  gear: "⚙", settings: "⚙",
  check: "✓",
  list: "≡",
  plus: "+", add: "+",
  trash: "✕", delete: "✕",
  star: "★",
  info: "ⓘ",
  number: "#", counter: "#",
  search: "🔍",
  edit: "✎",
  close: "✕",
  back: "←",
  forward: "→",
  calendar: "📅",
  clock: "🕐",
  user: "👤",
  mail: "✉",
  heart: "♥",
  bell: "🔔",
  share: "⤴",
  download: "⬇",
  upload: "⬆",
  refresh: "↻",
  filter: "▾",
  menu: "≡",
  more: "…",
  lock: "🔒",
  warning: "⚠",
};

const TONE_CLASS = { [EnumTone.muted]: "pl-tone-muted", [EnumTone.danger]: "pl-tone-danger", [EnumTone.success]: "pl-tone-success" };

/**
 * The SVG path of one pie wedge: center (cx, cy), radius r, start angle a0
 * (radians, 0 = 3 o'clock, clockwise) and its share `frac` of the circle.
 * A wedge over half the circle needs the large-arc flag.
 */
export function piePath(cx, cy, r, a0, frac) {
  if (frac > 0.9999) return `M ${cx} ${cy - r} A ${r} ${r} 0 1 1 ${cx - 0.001} ${cy - r} Z`;
  const a1 = a0 + frac * Math.PI * 2;
  const x1 = cx + r * Math.cos(a0), y1 = cy + r * Math.sin(a0);
  const x2 = cx + r * Math.cos(a1), y2 = cy + r * Math.sin(a1);
  return `M ${cx} ${cy} L ${x1} ${y1} A ${r} ${r} 0 ${frac > 0.5 ? 1 : 0} 1 ${x2} ${y2} Z`;
}

/** Elements that can hold the focus, in document order (see `saveFocus`). */
const FOCUSABLE = "input, textarea, select, button, a[href], [tabindex]";

const kindName = Object.fromEntries(Object.entries(ControlKind).map(([k, v]) => [v, k]));

class Node {
  constructor(id, kind) {
    this.id = id;
    this.kind = kind;
    this.props = new Map();
    this.text = null;
    this.children = [];
    this.parent = null;
    this.listeners = new Map(); // event code -> handler id
    this.el = null; // the DOM element, created lazily by render()
  }
}

/** The semantic tree + nav state that dom-renderer builds from the op stream. */
export class Tree {
  constructor() {
    this.nodes = new Map();
    this.roots = new Map(); // screen id -> root node id
    this.primaryScreens = []; // [{screen, node}] in mark-primary order
    this.currentScreen = 0;
    this.stack = []; // push/pop navigation (within a screen)
    this.onChange = () => {};
  }

  node(id) {
    return this.nodes.get(id);
  }

  apply(ops) {
    for (const op of ops) this.applyOne(op);
    this.onChange();
  }

  applyOne(op) {
    switch (op.op) {
      case "create":
        this.nodes.set(op.id, new Node(op.id, op.kind));
        break;
      case "remove": {
        const n = this.nodes.get(op.id);
        if (n?.parent) {
          const p = this.nodes.get(n.parent);
          p.children = p.children.filter((c) => c !== op.id);
        }
        this.nodes.delete(op.id);
        break;
      }
      case "insert":
      case "move": {
        const parent = this.nodes.get(op.parent);
        const child = this.nodes.get(op.id);
        if (!parent || !child) break;
        parent.children = parent.children.filter((c) => c !== op.id);
        const idx = op.before ? parent.children.indexOf(op.before) : -1;
        if (idx === -1) parent.children.push(op.id);
        else parent.children.splice(idx, 0, op.id);
        child.parent = op.parent;
        break;
      }
      case "set-prop": {
        const n = this.nodes.get(op.id);
        if (n) n.props.set(op.prop, op.value);
        break;
      }
      case "text": {
        const n = this.nodes.get(op.id);
        if (n) n.text = op.value;
        break;
      }
      case "listen": {
        const n = this.nodes.get(op.id);
        if (n) n.listeners.set(op.event, op.handler);
        break;
      }
      case "unlisten": {
        const n = this.nodes.get(op.id);
        if (n) n.listeners.delete(op.event);
        break;
      }
      case "set-root":
        this.roots.set(op.screen, op.id);
        break;
      case "navigate":
        this.applyNavigate(op);
        break;
      case "snapshot":
        break; // dev-only hot reload, not relevant to the web spike
      default:
        break;
    }
  }

  applyNavigate(op) {
    const NavKind = { PUSH: 0, REPLACE: 1, BACK: 2, SELECT_PRIMARY: 3, MARK_PRIMARY: 4 };
    switch (op.kind) {
      case NavKind.MARK_PRIMARY:
        if (!this.primaryScreens.some((p) => p.screen === op.screen)) {
          this.primaryScreens.push({ screen: op.screen });
        }
        break;
      case NavKind.SELECT_PRIMARY:
        this.currentScreen = op.screen;
        this.stack = [];
        break;
      case NavKind.PUSH:
        this.stack.push(op.screen);
        break;
      case NavKind.REPLACE:
        this.stack[this.stack.length - 1] = op.screen;
        break;
      case NavKind.BACK:
        this.stack.pop();
        break;
      default:
        break;
    }
  }

  /** The screen id currently on top (the pushed one, or the selected primary). */
  topScreen() {
    return this.stack.length ? this.stack[this.stack.length - 1] : this.currentScreen;
  }
}

function text(s) {
  return document.createTextNode(s ?? "");
}
function el(tag, className, attrs) {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (attrs) for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

const SVG_NS = "http://www.w3.org/2000/svg";
function svgEl(tag, attrs) {
  const e = document.createElementNS(SVG_NS, tag);
  if (attrs) for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
}

/** `Chart.data`/`series[].points` wire format (`wit/plinth/ui-api.toml`):
 * `"label\u0001value"` pairs joined with U+001F. */
function parseChartPoints(s) {
  if (!s) return [];
  return s
    .split("\u001f")
    .map((pair) => {
      const i = pair.indexOf("\u0001");
      if (i < 0) return null;
      const value = Number.parseFloat(pair.slice(i + 1));
      return { label: pair.slice(0, i), value: Number.isFinite(value) ? value : 0 };
    })
    .filter((p) => p !== null);
}

/** `Chart.series` wire format: series joined with U+001E, each
 * `"name\u0001points"`. */
function parseChartSeries(n) {
  const raw = n.props.get(Prop.series);
  if (raw) {
    return raw.split("\u001e").map((one) => {
      const i = one.indexOf("\u0001");
      const name = i < 0 ? one : one.slice(0, i);
      const points = i < 0 ? [] : parseChartPoints(one.slice(i + 1));
      return { name, points };
    });
  }
  const points = parseChartPoints(n.props.get(Prop.data) ?? "");
  return points.length ? [{ name: "", points }] : [];
}

/** The `<desc>`/AccessKit-equivalent summary of every value. */
function chartDescription(series) {
  return series
    .map((s) => {
      const body = s.points.map((p) => `${p.label}: ${p.value}`).join(", ");
      return s.name ? `${s.name}: ${body}` : body;
    })
    .join(". ");
}

/** Guesses a MIME type from an asset's file extension, for a blob URL. */
function assetMimeType(name) {
  const ext = name.split(".").pop()?.toLowerCase();
  const types = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", svg: "image/svg+xml", gif: "image/gif" };
  return types[ext] ?? "application/octet-stream";
}

/** Renders `tree` into `container`, re-rendering on every `tree.onChange`. */
export class DomRenderer {
  /** `assets` is a `Map<path, Uint8Array>` from `readPlnt` (SPEC.md §10.1, `<Image>`). */
  constructor(tree, container, app, assets = new Map()) {
    this.tree = tree;
    this.container = container;
    this.app = app;
    this.assets = assets;
    this.assetUrls = new Map(); // path -> blob: URL, built lazily and kept for the page's life
    tree.onChange = () => this.render();
    // A re-render during IME composition would end the composition; wait
    // for compositionend instead.
    this.composing = false;
    this.renderPending = false;
    container.addEventListener("compositionstart", () => { this.composing = true; });
    container.addEventListener("compositionend", () => {
      this.composing = false;
      if (this.renderPending) this.render();
    });
    // A click is a press and a release on the same element. A re-render
    // between them (a running timer re-renders many times a second) replaces
    // the element, and the browser sends no click. Wait while a pointer or
    // Space is down; render after the release, when the click has run.
    this.pressing = false;
    const press = () => { this.pressing = true; };
    const release = () => {
      if (!this.pressing) return;
      this.pressing = false;
      if (this.renderPending) setTimeout(() => { if (this.renderPending) this.render(); }, 0);
    };
    container.addEventListener("pointerdown", press);
    container.addEventListener("keydown", (e) => { if (e.key === " ") press(); });
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    window.addEventListener("keyup", (e) => { if (e.key === " ") release(); });
    window.addEventListener("blur", release);
  }

  assetUrl(path) {
    if (this.assetUrls.has(path)) return this.assetUrls.get(path);
    const bytes = this.assets.get(path);
    if (!bytes) return null;
    const url = URL.createObjectURL(new Blob([bytes], { type: assetMimeType(path) }));
    this.assetUrls.set(path, url);
    return url;
  }

  send(handler, eventCode, value) {
    this.app.onEvent({ kind: "ui", handler, event: eventCode, value });
  }

  /**
   * A `change` of a two-way bound `value`. The guest does not echo the value
   * back, so the tree keeps it here; otherwise the next re-render (from any
   * other prop change) writes the old value into the new element.
   */
  sendChange(n, handler, value) {
    n.props.set(Prop.value, value);
    this.send(handler, Event.change, value);
  }

  /**
   * The focused element, as the node id of its closest rendered node, the
   * index among that node's focusable descendants (-1: the node element
   * itself), and the text selection. `render()` replaces every element, so
   * without this the focus is lost after each change (one key in a TextField).
   */
  saveFocus() {
    const a = document.activeElement;
    if (!a || a === document.body || !this.container.contains(a)) return null;
    const host = a.closest("[data-pl-id]");
    if (!host) return null;
    const saved = {
      id: host.dataset.plId,
      index: host === a ? -1 : [...host.querySelectorAll(FOCUSABLE)].indexOf(a),
      start: null,
      end: null,
      dir: undefined,
    };
    try {
      saved.start = a.selectionStart;
      saved.end = a.selectionEnd;
      saved.dir = a.selectionDirection ?? undefined;
    } catch {
      // Inputs without a text selection (checkbox, range) throw in old engines.
    }
    return saved;
  }

  restoreFocus(saved) {
    if (!saved) return;
    const host = this.container.querySelector(`[data-pl-id="${saved.id}"]`);
    if (!host) return;
    const target = saved.index < 0 ? host : host.querySelectorAll(FOCUSABLE)[saved.index];
    if (!target) return;
    target.focus({ preventScroll: true });
    if (saved.start !== null && typeof target.setSelectionRange === "function") {
      const len = target.value?.length ?? 0;
      try {
        target.setSelectionRange(Math.min(saved.start, len), Math.min(saved.end, len), saved.dir);
      } catch {
        // Not a text input after all.
      }
    }
  }

  render() {
    if (this.composing || this.pressing) {
      this.renderPending = true;
      return;
    }
    this.renderPending = false;
    const focus = this.saveFocus();
    const scrollX = window.scrollX;
    const scrollY = window.scrollY;
    this.container.innerHTML = "";
    const nav = el("nav", "pl-nav", { "aria-label": "Screens" });
    for (const p of this.tree.primaryScreens) {
      const btn = el("button", "pl-nav-item");
      // The nav shows each screen's title, as the desktop shell does.
      const root = this.tree.nodes.get(this.tree.roots.get(p.screen));
      btn.textContent = root?.props.get(Prop.title) ?? `Screen ${p.screen}`;
      btn.disabled = p.screen === this.tree.currentScreen;
      if (p.screen === this.tree.currentScreen) btn.setAttribute("aria-current", "page");
      btn.addEventListener("click", () => {
        this.tree.currentScreen = p.screen;
        this.tree.stack = [];
        this.render();
      });
      nav.appendChild(btn);
    }
    const screenId = this.tree.topScreen();
    const rootId = this.tree.roots.get(screenId);
    const main = el("main", "pl-screen");
    if (rootId !== undefined) {
      const node = this.tree.node(rootId);
      if (node) main.appendChild(this.renderNode(node));
    }
    if (this.tree.primaryScreens.length) this.container.appendChild(nav);
    this.container.appendChild(main);
    // document.title follows the current screen, matching the desktop
    // window title (SPEC.md §6).
    const rootNode = rootId !== undefined ? this.tree.node(rootId) : null;
    const screenTitle = rootNode?.props.get(Prop.title);
    document.title = screenTitle ? `${screenTitle} — Plinth` : "Plinth web host";
    this.restoreFocus(focus);
    window.scrollTo(scrollX, scrollY);
  }

  /** Renders one node and tags its element with the node id (see `saveFocus`). */
  renderNode(n) {
    const dom = this.renderNodeElement(n);
    if (dom && dom.nodeType === 1) dom.dataset.plId = String(n.id);
    return dom;
  }

  renderNodeElement(n) {
    const kname = kindName[n.kind] ?? `kind${n.kind}`;
    switch (kname) {
      case "screen":
        return this.renderScreen(n);
      case "section":
        return this.renderSection(n);
      case "text":
        return this.renderText(n);
      case "heading":
        return this.renderHeading(n);
      case "button":
        return this.renderButton(n);
      case "textField":
        return this.renderTextField(n);
      case "toggle":
        return this.renderToggle(n);
      case "list":
        return this.renderList(n);
      case "row":
        return this.renderRow(n);
      case "group":
        return this.renderGroup(n);
      case "empty":
        return this.renderEmpty(n);
      case "checkbox":
        return this.renderCheckbox(n);
      case "textArea":
        return this.renderTextArea(n);
      case "slider":
        return this.renderSlider(n);
      case "numberField":
        return this.renderNumberField(n);
      case "picker":
        return this.renderPicker(n);
      case "progress":
        return this.renderProgress(n);
      case "badge":
        return this.renderBadge(n);
      case "tabs":
        return this.renderTabs(n);
      case "sheet":
      case "dialog":
        return this.renderSheetOrDialog(n, kname);
      case "menu":
        return this.renderMenu(n);
      case "grid":
        return this.renderGrid(n);
      case "action":
        return this.renderActionStandalone(n);
      case "image":
        return this.renderImage(n);
      case "icon":
        return this.renderIcon(n);
      case "datePicker":
        return this.renderDatePicker(n);
      case "chart":
        return this.renderChart(n);
      default:
        return this.renderChildren(n, el("div", `pl-${kname}`));
    }
  }

  renderChildren(n, container) {
    for (const cid of n.children) {
      const c = this.tree.node(cid);
      if (c) container.appendChild(this.renderNode(c));
    }
    return container;
  }

  renderScreen(n) {
    const container = el("section", "pl-screen-body");
    const header = el("div", "pl-screen-header");
    if (this.tree.stack.length > 0) {
      const back = el("button", "pl-back");
      back.setAttribute("aria-label", "Back");
      back.textContent = "← Back";
      back.addEventListener("click", () => {
        this.tree.stack.pop();
        this.render();
      });
      header.appendChild(back);
    }
    const title = n.props.get(Prop.title);
    if (title) header.appendChild(el("h1", "pl-title")).appendChild(text(title));
    container.appendChild(header);
    // Actions become `action`-kind child nodes (SPEC.md §6.3 UI API 1.2); render
    // them as a toolbar above the other children rather than inline.
    const actionChildren = n.children.map((cid) => this.tree.node(cid)).filter((c) => c && kindName[c.kind] === "action");
    if (actionChildren.length) {
      const toolbar = el("div", "pl-toolbar");
      for (const a of actionChildren) toolbar.appendChild(this.renderActionButton(a));
      container.appendChild(toolbar);
    }
    for (const cid of n.children) {
      const c = this.tree.node(cid);
      if (c && kindName[c.kind] !== "action") container.appendChild(this.renderNode(c));
    }
    return container;
  }

  /** An `Action` child's button, used in a toolbar (Screen/Menu) or a Dialog's action row. */
  renderActionButton(n) {
    const b = el("button", "pl-action");
    b.textContent = n.props.get(Prop.label) ?? "";
    const role = n.props.get(Prop.role);
    if (role && role.enum === 2) b.classList.add("pl-action-destructive");
    const handler = n.listeners.get(Event.press);
    if (handler !== undefined) {
      b.addEventListener("click", () => {
        const destructive = role && role.enum === 2;
        const confirmProp = n.props.get(Prop.confirm);
        const needsConfirm = destructive && confirmProp !== false;
        if (needsConfirm && !window.confirm(`${b.textContent}?`)) return;
        this.send(handler, Event.press, null);
      });
    }
    return b;
  }

  /** An `Action` rendered where no container (Screen/Dialog/Menu) claimed it as chrome. */
  renderActionStandalone(n) {
    return this.renderActionButton(n);
  }

  renderSection(n) {
    const container = el("fieldset", "pl-section");
    const title = n.props.get(Prop.title);
    if (title) {
      const legend = el("legend", "pl-section-title");
      legend.appendChild(text(title));
      container.appendChild(legend);
    }
    return this.renderChildren(n, container);
  }

  renderText(n) {
    const p = el("p", "pl-text");
    p.appendChild(text(n.text));
    return p;
  }

  renderHeading(n) {
    const level = n.props.get(Prop.level) || 2;
    const h = el(`h${Math.min(Math.max(level, 1), 6)}`, "pl-heading");
    h.appendChild(text(n.text ?? n.props.get(Prop.title)));
    return h;
  }

  renderButton(n) {
    const b = el("button", "pl-button");
    b.textContent = n.props.get(Prop.label) ?? n.text ?? "";
    if (n.props.get(Prop.disabled)) b.disabled = true;
    const handler = n.listeners.get(Event.press);
    if (handler !== undefined) {
      b.addEventListener("click", () => this.send(handler, Event.press, null));
    }
    return b;
  }

  renderTextField(n) {
    const wrap = el("label", "pl-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const input = el("input", "pl-input", { type: "text" });
    input.value = n.props.get(Prop.value) ?? "";
    input.placeholder = n.props.get(Prop.placeholder) ?? "";
    const changeHandler = n.listeners.get(Event.change);
    const submitHandler = n.listeners.get(Event.submit);
    if (changeHandler !== undefined) {
      input.addEventListener("input", () => this.sendChange(n, changeHandler, input.value));
    }
    if (submitHandler !== undefined) {
      input.addEventListener("keydown", (e) => {
        if (e.key === "Enter") this.send(submitHandler, Event.submit, input.value);
      });
    }
    wrap.appendChild(input);
    return wrap;
  }

  renderToggle(n) {
    const wrap = el("label", "pl-toggle");
    const input = el("input", "pl-checkbox", { type: "checkbox" });
    input.checked = !!n.props.get(Prop.value);
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      input.addEventListener("change", () => this.sendChange(n, handler, input.checked));
    }
    wrap.appendChild(input);
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-toggle-label")).appendChild(text(label));
    return wrap;
  }

  renderList(n) {
    return this.renderChildren(n, el("div", "pl-list", { role: "list" }));
  }

  renderRow(n) {
    const handler = n.listeners.get(Event.press);
    const row = el("div", "pl-row", { role: "listitem" });
    const header = el("div", "pl-row-header");
    const title = n.props.get(Prop.title);
    const subtitle = n.props.get(Prop.subtitle);
    if (title) header.appendChild(el("span", "pl-row-title")).appendChild(text(title));
    if (subtitle) header.appendChild(el("span", "pl-row-subtitle")).appendChild(text(subtitle));
    row.appendChild(header);
    const trailing = n.props.get(Prop.trailing);
    if (trailing) row.appendChild(el("span", "pl-row-trailing")).appendChild(text(trailing));
    const actions = el("div", "pl-row-actions");
    this.renderChildren(n, actions);
    row.appendChild(actions);
    if (handler !== undefined) {
      // A pressable Row behaves like a button for keyboard users (Tab to
      // focus, Enter/Space to activate), matching the desktop renderer.
      row.classList.add("pl-row-pressable");
      row.tabIndex = 0;
      row.addEventListener("click", () => this.send(handler, Event.press, null));
      row.addEventListener("keydown", (e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          this.send(handler, Event.press, null);
        }
      });
    }
    return row;
  }

  renderGroup(n) {
    return this.renderChildren(n, el("div", "pl-group"));
  }

  renderEmpty(n) {
    const wrap = el("div", "pl-empty");
    const title = n.props.get(Prop.title);
    const message = n.props.get(Prop.message);
    if (title) wrap.appendChild(el("p", "pl-empty-title")).appendChild(text(title));
    if (message) wrap.appendChild(el("p", "pl-empty-message")).appendChild(text(message));
    return wrap;
  }

  renderCheckbox(n) {
    const wrap = el("label", "pl-checkbox-field");
    const input = el("input", "pl-checkbox", { type: "checkbox" });
    input.checked = !!n.props.get(Prop.value);
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      input.addEventListener("change", () => this.sendChange(n, handler, input.checked));
    }
    wrap.appendChild(input);
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-checkbox-label")).appendChild(text(label));
    return wrap;
  }

  renderTextArea(n) {
    const wrap = el("label", "pl-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const input = el("textarea", "pl-textarea");
    input.value = n.props.get(Prop.value) ?? "";
    input.placeholder = n.props.get(Prop.placeholder) ?? "";
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      input.addEventListener("input", () => this.sendChange(n, handler, input.value));
    }
    wrap.appendChild(input);
    return wrap;
  }

  renderSlider(n) {
    const wrap = el("label", "pl-field pl-slider-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const input = el("input", "pl-slider", { type: "range" });
    input.min = String(n.props.get(Prop.min) ?? 0);
    input.max = String(n.props.get(Prop.max) ?? 100);
    const step = n.props.get(Prop.step);
    if (step != null) input.step = String(step);
    input.value = String(n.props.get(Prop.value) ?? 0);
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      // Pointer drag on the track is native <input type=range> behavior,
      // snapped to `step` by the browser (SPEC.md §6.3).
      input.addEventListener("input", () => this.sendChange(n, handler, Number(input.value)));
    }
    wrap.appendChild(input);
    return wrap;
  }

  renderNumberField(n) {
    const wrap = el("label", "pl-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const input = el("input", "pl-input", { type: "number" });
    const min = n.props.get(Prop.min);
    const max = n.props.get(Prop.max);
    const step = n.props.get(Prop.step);
    if (min != null) input.min = String(min);
    if (max != null) input.max = String(max);
    if (step != null) input.step = String(step);
    input.value = String(n.props.get(Prop.value) ?? 0);
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      input.addEventListener("input", () => {
        const v = input.valueAsNumber;
        if (!Number.isNaN(v)) this.sendChange(n, handler, v);
      });
    }
    wrap.appendChild(input);
    return wrap;
  }

  /** `Picker.options` arrive joined with U+001F (SPEC.md §6.3 UI API 1.2). */
  renderPicker(n) {
    const options = (n.props.get(Prop.options) ?? "").split("\u001f").filter((s) => s.length > 0);
    const wrap = el("div", "pl-field pl-picker-field");
    const label = n.props.get(Prop.label);
    const current = n.props.get(Prop.value) ?? "";
    const handler = n.listeners.get(Event.change);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    if (options.length <= 4) {
      // A segmented control (SPEC.md §6.3: four options or less).
      const group = el("div", "pl-segmented", { role: "radiogroup", "aria-label": label ?? "" });
      for (const opt of options) {
        const optLabel = el("label", "pl-segment");
        const radio = el("input", "pl-segment-input", { type: "radio", name: `picker-${n.id}` });
        radio.checked = opt === current;
        if (handler !== undefined) {
          radio.addEventListener("change", () => this.sendChange(n, handler, opt));
        }
        optLabel.appendChild(radio);
        optLabel.appendChild(el("span", "pl-segment-label")).appendChild(text(opt));
        group.appendChild(optLabel);
      }
      wrap.appendChild(group);
    } else {
      const select = el("select", "pl-select", label ? { "aria-label": label } : {});
      for (const opt of options) {
        const o = el("option");
        o.value = opt;
        o.textContent = opt;
        if (opt === current) o.selected = true;
        select.appendChild(o);
      }
      if (handler !== undefined) {
        select.addEventListener("change", () => this.sendChange(n, handler, select.value));
      }
      wrap.appendChild(select);
    }
    return wrap;
  }

  renderProgress(n) {
    const wrap = el("div", "pl-progress-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const value = n.props.get(Prop.value);
    const attrs = value == null ? {} : { value: String(value), max: "1" };
    const progress = el("progress", "pl-progress", attrs);
    if (label) progress.setAttribute("aria-label", label);
    wrap.appendChild(progress);
    return wrap;
  }

  renderBadge(n) {
    const b = el("span", "pl-badge");
    b.textContent = n.props.get(Prop.label) ?? n.props.get(Prop.title) ?? "";
    return b;
  }

  /** `<Icon>` (SPEC.md §6.3, UI API 1.4): a glyph from the runtime icon
   * set. Decorative by default (hidden from screen readers); `label`
   * gives it an accessible name. */
  renderIcon(n) {
    const name = n.props.get(Prop.icon) ?? "";
    const label = n.props.get(Prop.label);
    const tone = n.props.get(Prop.tone)?.enum;
    const cls = ["pl-icon", TONE_CLASS[tone]].filter(Boolean).join(" ");
    const span = el("span", cls);
    span.textContent = ICON_GLYPH[name] ?? "•";
    if (label) {
      span.setAttribute("role", "img");
      span.setAttribute("aria-label", label);
    } else {
      span.setAttribute("aria-hidden", "true");
    }
    return span;
  }

  /** `<DatePicker>` (SPEC.md §6.3, UI API 1.4): a native date/time input,
   * two-way bound exactly like `TextField` (the host sets `input.value`
   * first, then sends `change`, with no echo back from the guest). */
  renderDatePicker(n) {
    const wrap = el("label", "pl-field");
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-field-label")).appendChild(text(label));
    const mode = n.props.get(Prop.mode)?.enum ?? EnumDatePickerMode.date;
    const type = mode === EnumDatePickerMode.time ? "time" : mode === EnumDatePickerMode.datetime ? "datetime-local" : "date";
    const input = el("input", "pl-input", { type });
    input.value = n.props.get(Prop.value) ?? "";
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) {
      input.addEventListener("change", () => this.sendChange(n, handler, input.value));
    }
    wrap.appendChild(input);
    return wrap;
  }

  /** `<Image>` (SPEC.md §6.3, §10.1): an asset from the package. Sized by
   * `aspect`, not pixels. A missing asset falls back to a placeholder box
   * with the `alt` text, so a broken image never breaks accessibility. */
  renderImage(n) {
    const src = n.props.get(Prop.src) ?? "";
    const alt = n.props.get(Prop.alt) ?? "";
    const aspectProp = n.props.get(Prop.aspect);
    const aspectValue = aspectProp?.enum ?? EnumAspect.square;
    const ratio = ASPECT_RATIO[aspectValue] ?? ASPECT_RATIO[EnumAspect.square];
    const aspectClass = aspectValue === EnumAspect.wide ? "pl-image-wide" : aspectValue === EnumAspect.tall ? "pl-image-tall" : "pl-image-square";
    const url = this.assetUrl(src);
    const wrap = el("div", `pl-image ${aspectClass}`, { style: `aspect-ratio: ${ratio}` });
    if (url) {
      const img = el("img", "pl-image-img", { alt, src: url });
      wrap.appendChild(img);
    } else {
      wrap.classList.add("pl-image-placeholder");
      wrap.setAttribute("role", "img");
      wrap.setAttribute("aria-label", alt);
      wrap.appendChild(el("span", "pl-image-placeholder-text")).appendChild(text(alt));
    }
    return wrap;
  }

  /** `<Chart>` (SPEC.md §6.3, UI API 1.5): inline SVG with `<title>`/`<desc>`
   * for the accessible name and summary, plus a visually hidden `<table>`
   * so a screen reader gets every value. The runtime (this renderer), not
   * the app, picks the colors (the CSS chart palette, derived from the
   * accent and neutral tokens), height and axis ticks. */
  renderChart(n) {
    const label = n.props.get(Prop.label) ?? "";
    const kind = n.props.get(Prop.chartKind)?.enum ?? EnumChartKind.bar;
    const series = parseChartSeries(n);
    const wrap = el("div", "pl-chart");

    if (series.length === 0 || series.every((s) => s.points.length === 0)) {
      wrap.setAttribute("role", "figure");
      wrap.setAttribute("aria-label", label);
      const empty = el("div", "pl-chart-empty");
      empty.appendChild(text("No data"));
      wrap.appendChild(empty);
      return wrap;
    }

    wrap.appendChild(this.renderChartSvg(series, kind, label));
    const showLegend = series.length > 1 || (kind === EnumChartKind.pie && series[0].points.length > 1);
    if (showLegend) {
      const names = kind === EnumChartKind.pie ? series[0].points.map((p) => p.label) : series.map((s) => s.name);
      wrap.appendChild(this.renderChartLegend(names));
    }
    wrap.appendChild(this.renderChartTable(series));
    return wrap;
  }

  renderChartSvg(series, kind, label) {
    const W = 300, H = 150;
    const svg = svgEl("svg", { class: "pl-chart-svg", viewBox: `0 0 ${W} ${H}`, role: "img", "aria-labelledby": "" });
    const titleId = `pl-chart-title-${Math.random().toString(36).slice(2)}`;
    const descId = `pl-chart-desc-${Math.random().toString(36).slice(2)}`;
    svg.setAttribute("aria-labelledby", `${titleId} ${descId}`);
    const title = svgEl("title", { id: titleId });
    title.appendChild(text(label));
    svg.appendChild(title);
    const desc = svgEl("desc", { id: descId });
    desc.appendChild(text(chartDescription(series)));
    svg.appendChild(desc);

    if (kind === EnumChartKind.pie) {
      const points = series[0].points;
      const total = points.reduce((s, p) => s + Math.abs(p.value), 0) || 1;
      const cx = W / 2, cy = H / 2, r = Math.min(W, H) / 2 - 6;
      let angle = -Math.PI / 2;
      points.forEach((p, i) => {
        const frac = Math.abs(p.value) / total;
        if (frac <= 0) return;
        svg.appendChild(svgEl("path", { d: piePath(cx, cy, r, angle, frac), class: `pl-chart-c${i % 6}` }));
        angle += frac * Math.PI * 2;
      });
      return svg;
    }

    const max = Math.max(1e-9, ...series.flatMap((s) => s.points.map((p) => Math.abs(p.value))));
    const labels = series[0].points.map((p) => p.label);
    const padL = 4, padR = 4, padB = 16, padT = 6;
    const plotW = W - padL - padR, plotH = H - padT - padB;
    const n = Math.max(1, labels.length);

    if (kind === EnumChartKind.line) {
      series.forEach((s, si) => {
        const pts = s.points.map((p, i) => {
          const x = padL + (n === 1 ? plotW / 2 : (i / (n - 1)) * plotW);
          const y = padT + plotH - (Math.abs(p.value) / max) * plotH;
          return `${x},${y}`;
        });
        svg.appendChild(svgEl("polyline", { points: pts.join(" "), class: `pl-chart-line pl-chart-c${si % 6}`, fill: "none" }));
        s.points.forEach((p, i) => {
          const x = padL + (n === 1 ? plotW / 2 : (i / (n - 1)) * plotW);
          const y = padT + plotH - (Math.abs(p.value) / max) * plotH;
          svg.appendChild(svgEl("circle", { cx: x, cy: y, r: 2.5, class: `pl-chart-c${si % 6}` }));
        });
      });
    } else {
      const groupW = plotW / n;
      const barW = groupW / (series.length + 1);
      labels.forEach((lbl, i) => {
        series.forEach((s, si) => {
          const v = s.points[i]?.value ?? 0;
          const h = (Math.abs(v) / max) * plotH;
          const x = padL + i * groupW + barW * (si + 0.5);
          const y = padT + plotH - h;
          svg.appendChild(svgEl("rect", { x, y, width: barW * 0.85, height: Math.max(h, 1), class: `pl-chart-c${si % 6}` }));
        });
      });
    }
    labels.forEach((lbl, i) => {
      const x = padL + (n === 1 ? plotW / 2 : (i + 0.5) * (plotW / n));
      const t = svgEl("text", { x, y: H - 3, class: "pl-chart-axis-label", "text-anchor": "middle" });
      t.appendChild(text(lbl));
      svg.appendChild(t);
    });
    return svg;
  }

  renderChartLegend(names) {
    const legend = el("ul", "pl-chart-legend");
    names.forEach((name, i) => {
      const item = el("li", `pl-chart-legend-item pl-chart-c${i % 6}`);
      item.appendChild(el("span", "pl-chart-legend-swatch"));
      item.appendChild(text(name));
      legend.appendChild(item);
    });
    return legend;
  }

  /** A visually hidden data table: a screen reader gets the exact numbers
   * without relying on the drawing. */
  renderChartTable(series) {
    const table = el("table", "pl-chart-table pl-sr-only");
    const multi = series.length > 1 && series.some((s) => s.name);
    const labels = series[0]?.points.map((p) => p.label) ?? [];
    const head = el("tr");
    head.appendChild(el("th"));
    if (multi) {
      series.forEach((s) => {
        const th = el("th");
        th.appendChild(text(s.name));
        head.appendChild(th);
      });
    } else {
      head.appendChild(el("th"));
    }
    table.appendChild(el("thead")).appendChild(head);
    const body = el("tbody");
    labels.forEach((lbl, i) => {
      const row = el("tr");
      const th = el("th");
      th.appendChild(text(lbl));
      row.appendChild(th);
      if (multi) {
        series.forEach((s) => {
          const td = el("td");
          td.appendChild(text(String(s.points[i]?.value ?? "")));
          row.appendChild(td);
        });
      } else {
        const td = el("td");
        td.appendChild(text(String(series[0].points[i]?.value ?? "")));
        row.appendChild(td);
      }
      body.appendChild(row);
    });
    table.appendChild(body);
    return table;
  }

  /** `Tabs.items` arrive joined with U+001F; `value` is the selected item's text. */
  renderTabs(n) {
    const items = (n.props.get(Prop.items) ?? "").split("\u001f").filter((s) => s.length > 0);
    const current = n.props.get(Prop.value) ?? items[0];
    const handler = n.listeners.get(Event.change);
    const list = el("div", "pl-tabs", { role: "tablist" });
    const tabs = [];
    for (const item of items) {
      const selected = item === current;
      const tab = el("button", "pl-tab", { role: "tab", "aria-selected": selected ? "true" : "false" });
      tab.tabIndex = selected ? 0 : -1;
      tab.textContent = item;
      if (selected) tab.classList.add("pl-tab-selected");
      if (handler !== undefined) {
        tab.addEventListener("click", () => this.sendChange(n, handler, item));
      }
      tabs.push(tab);
      list.appendChild(tab);
    }
    // Arrow-key roving tabindex between tabs (SPEC.md §6.3): Left/Right
    // move focus and activate, matching the desktop Tabs control.
    tabs.forEach((tab, i) => {
      tab.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
        e.preventDefault();
        const next = e.key === "ArrowRight" ? (i + 1) % tabs.length : (i - 1 + tabs.length) % tabs.length;
        tabs[next].focus();
        if (handler !== undefined) this.sendChange(n, handler, items[next]);
      });
    });
    return this.renderChildren(n, (() => {
      const wrap = el("div", "pl-tabs-wrap");
      wrap.appendChild(list);
      return wrap;
    })());
  }

  /** `Sheet`/`Dialog` bind `open` both ways (SPEC.md §6.3 UI API 1.2): a native `<dialog>`. */
  renderSheetOrDialog(n, kname) {
    const dialog = el("dialog", `pl-${kname}`);
    const title = n.props.get(Prop.title);
    if (title) dialog.appendChild(el("h2", "pl-title")).appendChild(text(title));
    for (const cid of n.children) {
      const c = this.tree.node(cid);
      if (c) dialog.appendChild(this.renderNode(c));
    }
    const closeHandler = n.listeners.get(Event.close);
    const onDialogClose = () => {
      if (closeHandler !== undefined) this.send(closeHandler, Event.close, null);
    };
    dialog.addEventListener("close", onDialogClose);
    dialog.addEventListener("cancel", onDialogClose); // Escape
    const open = !!n.props.get(Prop.value);
    // Open/close after the element is in the document (showModal needs that).
    queueMicrotask(() => {
      if (open && !dialog.open) dialog.showModal();
      if (!open && dialog.open) dialog.close();
    });
    return dialog;
  }

  /** `Menu` is an anchored popover (SPEC.md §6.3): label button + its `action` children. */
  renderMenu(n) {
    const wrap = el("div", "pl-menu");
    const trigger = el("button", "pl-menu-trigger", { "aria-haspopup": "menu", "aria-expanded": "false" });
    trigger.textContent = n.props.get(Prop.label) ?? "";
    const popover = el("div", "pl-menu-popover", { role: "menu" });
    popover.hidden = true;
    const actions = n.children.map((cid) => this.tree.node(cid)).filter((c) => c && kindName[c.kind] === "action");
    const items = [];
    for (const a of actions) {
      const item = this.renderActionButton(a);
      item.setAttribute("role", "menuitem");
      item.classList.add("pl-menu-item");
      item.tabIndex = -1;
      item.addEventListener("click", () => closeMenu());
      items.push(item);
      popover.appendChild(item);
    }
    const closeMenu = () => {
      popover.hidden = true;
      trigger.setAttribute("aria-expanded", "false");
      document.removeEventListener("click", onDocClick);
      trigger.focus();
    };
    const onDocClick = (e) => {
      if (!wrap.contains(e.target)) closeMenu();
    };
    popover.addEventListener("keydown", (e) => {
      const i = items.indexOf(document.activeElement);
      if (e.key === "Escape") {
        e.preventDefault();
        closeMenu();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        items[(i + 1 + items.length) % items.length]?.focus();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        items[(i - 1 + items.length) % items.length]?.focus();
      }
    });
    trigger.addEventListener("click", () => {
      const willOpen = popover.hidden;
      popover.hidden = !willOpen;
      trigger.setAttribute("aria-expanded", willOpen ? "true" : "false");
      if (willOpen) {
        document.addEventListener("click", onDocClick);
        items[0]?.focus();
      }
    });
    wrap.appendChild(trigger);
    wrap.appendChild(popover);
    return wrap;
  }

  /** CSS grid, column count by width class (SPEC.md §6.3: 2/3/4 columns). */
  renderGrid(n) {
    // Not every Grid child is a Row (`role=listitem`), so the grid itself
    // cannot always take `role=list` (that requires only listitem children,
    // SPEC.md §6.3's "2/3/4 columns" layout is otherwise unconstrained).
    return this.renderChildren(n, el("div", "pl-grid"));
  }
}
