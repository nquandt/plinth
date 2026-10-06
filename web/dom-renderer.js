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
    const title = n.props.get(Prop.title);
    if (title) container.appendChild(el("h1", "pl-title")).appendChild(text(title));
    return this.renderChildren(n, container);
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
}
