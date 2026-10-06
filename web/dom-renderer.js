// A minimal DOM renderer for the op buffer (SPEC.md §8.2), for the web
// host spike. Applies ops to a semantic tree of plain objects, then
// renders Screen/Section/Text/Heading/Button/TextField/Toggle/List/Row/
// Group/Empty (plus a nav shell for primary screens) as plain semantic
// HTML, and sends `ui` events (press/change) back through `on-event`.
//
// This file is DOM-only: it does not touch wasm directly. `app.onCommit`
// (from plinth-web.js) feeds it ops; it calls `app.onEvent(...)` back.

import { ControlKind, Prop, Event } from "./ui-api.js";

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

/** Renders `tree` into `container`, re-rendering on every `tree.onChange`. */
export class DomRenderer {
  constructor(tree, container, app) {
    this.tree = tree;
    this.container = container;
    this.app = app;
    tree.onChange = () => this.render();
  }

  send(handler, eventCode, value) {
    this.app.onEvent({ kind: "ui", handler, event: eventCode, value });
  }

  render() {
    this.container.innerHTML = "";
    const nav = el("nav", "pl-nav");
    for (const p of this.tree.primaryScreens) {
      const btn = el("button", "pl-nav-item");
      // The nav shows each screen's title, as the desktop shell does.
      const root = this.tree.nodes.get(this.tree.roots.get(p.screen));
      btn.textContent = root?.props.get(Prop.title) ?? `Screen ${p.screen}`;
      btn.disabled = p.screen === this.tree.currentScreen;
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
  }

  renderNode(n) {
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
      input.addEventListener("input", () => this.send(changeHandler, Event.change, input.value));
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
      input.addEventListener("change", () => this.send(handler, Event.change, input.checked));
    }
    wrap.appendChild(input);
    const label = n.props.get(Prop.label);
    if (label) wrap.appendChild(el("span", "pl-toggle-label")).appendChild(text(label));
    return wrap;
  }

  renderList(n) {
    return this.renderChildren(n, el("div", "pl-list"));
  }

  renderRow(n) {
    const row = el("div", "pl-row");
    const header = el("div", "pl-row-header");
    const title = n.props.get(Prop.title);
    const subtitle = n.props.get(Prop.subtitle);
    if (title) header.appendChild(el("span", "pl-row-title")).appendChild(text(title));
    if (subtitle) header.appendChild(el("span", "pl-row-subtitle")).appendChild(text(subtitle));
    row.appendChild(header);
    const actions = el("div", "pl-row-actions");
    this.renderChildren(n, actions);
    row.appendChild(actions);
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
      input.addEventListener("change", () => this.send(handler, Event.change, input.checked));
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
      input.addEventListener("input", () => this.send(handler, Event.change, input.value));
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
      input.addEventListener("input", () => this.send(handler, Event.change, Number(input.value)));
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
        if (!Number.isNaN(v)) this.send(handler, Event.change, v);
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
          radio.addEventListener("change", () => this.send(handler, Event.change, opt));
        }
        optLabel.appendChild(radio);
        optLabel.appendChild(el("span", "pl-segment-label")).appendChild(text(opt));
        group.appendChild(optLabel);
      }
      wrap.appendChild(group);
    } else {
      const select = el("select", "pl-select");
      for (const opt of options) {
        const o = el("option");
        o.value = opt;
        o.textContent = opt;
        if (opt === current) o.selected = true;
        select.appendChild(o);
      }
      if (handler !== undefined) {
        select.addEventListener("change", () => this.send(handler, Event.change, select.value));
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

  /** `Tabs.items` arrive joined with U+001F; `value` is the selected item's text. */
  renderTabs(n) {
    const items = (n.props.get(Prop.items) ?? "").split("\u001f").filter((s) => s.length > 0);
    const current = n.props.get(Prop.value) ?? items[0];
    const handler = n.listeners.get(Event.change);
    const list = el("div", "pl-tabs", { role: "tablist" });
    for (const item of items) {
      const tab = el("button", "pl-tab", { role: "tab", "aria-selected": item === current ? "true" : "false" });
      tab.textContent = item;
      if (item === current) tab.classList.add("pl-tab-selected");
      if (handler !== undefined) {
        tab.addEventListener("click", () => this.send(handler, Event.change, item));
      }
      list.appendChild(tab);
    }
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
    const trigger = el("button", "pl-menu-trigger", { "aria-haspopup": "menu" });
    trigger.textContent = n.props.get(Prop.label) ?? "";
    const popover = el("div", "pl-menu-popover", { role: "menu" });
    popover.hidden = true;
    const actions = n.children.map((cid) => this.tree.node(cid)).filter((c) => c && kindName[c.kind] === "action");
    for (const a of actions) {
      const item = this.renderActionButton(a);
      item.setAttribute("role", "menuitem");
      item.classList.add("pl-menu-item");
      item.addEventListener("click", () => {
        popover.hidden = true;
      });
      popover.appendChild(item);
    }
    const close = (e) => {
      if (!wrap.contains(e.target)) {
        popover.hidden = true;
        document.removeEventListener("click", close);
      }
    };
    trigger.addEventListener("click", () => {
      popover.hidden = !popover.hidden;
      if (!popover.hidden) document.addEventListener("click", close);
    });
    wrap.appendChild(trigger);
    wrap.appendChild(popover);
    return wrap;
  }

  /** CSS grid, column count by width class (SPEC.md §6.3: 2/3/4 columns). */
  renderGrid(n) {
    return this.renderChildren(n, el("div", "pl-grid"));
  }
}
