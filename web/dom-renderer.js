// The DOM renderer of the web host (SPEC.md §8.2, M5). `Tree` applies the
// op buffer to a semantic tree of plain objects and records what changed.
// `DomRenderer` keeps ONE DOM element per tree node (a "view", indexed by
// node id) and, after each commit, updates only the views of the nodes
// that changed: their props, their text and the order of their children.
// An element is made again only when its node's kind changes or when a
// control needs a different element (a Heading level). Thus the focus, the
// caret, the text selection and the hover state stay across commits, as
// they do for any static page.
//
// This file is DOM-only: it does not touch wasm directly. `app.onCommit`
// (from plinth-web.js) feeds `Tree.apply`; the renderer calls
// `app.onEvent(...)` back.

import {
  ControlKind,
  Prop,
  Event,
  EnumAspect,
  EnumTone,
  EnumDatePickerMode,
  EnumChartKind,
  EnumAxis,
  EnumCrossAlign,
  EnumJustify,
  EnumFraction,
  EnumColor,
  EnumRadius,
  EnumTextSize,
  EnumWeight,
  EnumPressableRole,
  EnumTextAlign,
  EnumPosition,
} from "./ui-api.js";

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

const kindName = Object.fromEntries(Object.entries(ControlKind).map(([k, v]) => [v, k]));

// -- Chart geometry (shared with the desktop's `crates/plinth-ui/src/chart.rs`) --

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

/**
 * A "nice" y scale `{ lo, hi, step }` that covers every value and zero
 * (steps of 1, 2, 2.5 or 5 times a power of ten, about four intervals).
 * The same as `Scale::for_values` on the desktop.
 */
export function niceScale(values) {
  let min = 0, max = 0;
  for (const v of values) {
    if (!Number.isFinite(v)) continue;
    min = Math.min(min, v);
    max = Math.max(max, v);
  }
  if (max - min < 1e-12) return { lo: 0, hi: 1, step: 0.25 };
  const raw = (max - min) / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw * (1 - 1e-9)) ?? 10 * mag;
  return { lo: Math.floor(min / step) * step, hi: Math.ceil(max / step) * step, step };
}

/** Where `v` falls between the scale's `lo` (0) and `hi` (1). */
export function scaleFrac(scale, v) {
  return Math.min(1, Math.max(0, (v - scale.lo) / (scale.hi - scale.lo)));
}

/**
 * The vertical extent of a bar for `v` as `[from, to]` fractions of the plot
 * height, measured from the bottom. A bar starts at the zero line: up for
 * a positive value, down for a negative one. Zero (or not finite): `null`,
 * no bar. The same as `bar_span` on the desktop.
 */
export function barSpan(scale, v) {
  if (!Number.isFinite(v) || v === 0) return null;
  const z = scaleFrac(scale, 0), f = scaleFrac(scale, v);
  return [Math.min(z, f), Math.max(z, f)];
}

/**
 * The pie legend: each point that has a wedge (a finite, non-zero value),
 * with its index (its color) and its share as a whole percent ("<1%" for
 * a small share that rounds to 0). The same as `pie_legend` on the desktop.
 */
export function pieLegend(points) {
  const mag = (v) => (Number.isFinite(v) ? Math.abs(v) : 0);
  const total = points.reduce((s, p) => s + mag(p.value), 0);
  if (total <= 0) return [];
  const out = [];
  points.forEach((p, index) => {
    const v = mag(p.value);
    if (v <= 0) return;
    const pct = Math.round((v / total) * 100);
    out.push({ index, label: p.label, percent: pct === 0 ? "<1%" : `${pct}%` });
  });
  return out;
}

// -- The semantic tree -----------------------------------------------------

class Node {
  constructor(id, kind) {
    this.id = id;
    this.kind = kind;
    this.props = new Map();
    this.text = null;
    this.children = [];
    this.parent = 0;
    this.listeners = new Map(); // event code -> handler id
  }
}

/**
 * The semantic tree + nav state that dom-renderer builds from the op stream.
 * Besides the tree, it records what each commit changed (`takeChanges`), so
 * the renderer touches only those nodes.
 */
// -- UI API 1.6: Level 2 styled primitives (docs/UI-ADVANCED.md) ----------

/** One spacing unit in CSS pixels (the same as the desktop host). */
export const UNIT = 4;
const MAX_UNITS = 512;

// UI API 1.11: sizes and insets can be fractional units (the compiler sends
// whole numbers for the other props).
const units = (v) => `${Number.isFinite(v) ? Math.min(Math.max(v, 0), MAX_UNITS) * UNIT : 0}px`;

/** Color tokens as the CSS custom properties of style.css. */
const COLOR_VARS = {
  [EnumColor.background]: "--pl-bg",
  [EnumColor.surface]: "--pl-surface",
  [EnumColor.surfaceAlt]: "--pl-surface-alt",
  [EnumColor.accent]: "--pl-accent",
  [EnumColor.danger]: "--pl-danger",
  [EnumColor.success]: "--pl-success",
  [EnumColor.text]: "--pl-text",
  [EnumColor.textMuted]: "--pl-muted",
  [EnumColor.onAccent]: "--pl-accent-text",
  [EnumColor.border]: "--pl-border",
  [EnumColor.hover]: "--pl-hover",
  [EnumColor.selected]: "--pl-selected",
};
/**
 * The CSS value of a color token. `text`: the token colors text (`fg`); the
 * accent then uses its text variant, which is lighter in dark mode (as the
 * desktop's dark accents are), so that accent text stays legible.
 */
export const colorVar = (token, text = false) => {
  if (text && token === EnumColor.accent) return "var(--pl-accent-fg)";
  return COLOR_VARS[token] ? `var(${COLOR_VARS[token]})` : null;
};

const FRACTIONS = {
  [EnumFraction.auto]: "auto",
  [EnumFraction.full]: "100%",
  [EnumFraction.half]: "50%",
  [EnumFraction.third]: "33.3333%",
  [EnumFraction.twoThirds]: "66.6667%",
  [EnumFraction.quarter]: "25%",
  [EnumFraction.threeQuarters]: "75%",
};

/** A size prop: spacing units if the int prop is set, else the fraction, else null. */
function sizeOf(props, unitsProp, fractionProp) {
  const n = props.get(unitsProp);
  if (typeof n === "number") return units(n);
  return FRACTIONS[props.get(fractionProp)?.enum] ?? null;
}

const TEXT_SIZES = {
  [EnumTextSize.xs]: "0.75rem",
  [EnumTextSize.sm]: "0.875rem",
  [EnumTextSize.md]: "1rem",
  [EnumTextSize.lg]: "1.125rem",
  [EnumTextSize.xl]: "1.25rem",
  [EnumTextSize.xxl]: "1.5rem",
};
const WEIGHTS = { [EnumWeight.regular]: "400", [EnumWeight.medium]: "500", [EnumWeight.semibold]: "600", [EnumWeight.bold]: "700" };

/**
 * The inline CSS of a Level 2 primitive (`Box`, `Pressable`, `Scroll`,
 * `Span`) from its props: the same mapping as
 * `crates/plinth-ui/src/primitives.rs`. Returns `{ property: value }`.
 */
export function primitiveStyle(n, partial = false) {
  const p = n.props;
  const en = (prop) => p.get(prop)?.enum; // an enum prop is { enum: value }
  const has = (prop) => p.has(prop);
  const css = {};
  const grow = p.get(Prop.grow);
  if (typeof grow === "number" && grow > 0) {
    css.flex = `${Math.min(Math.trunc(grow), 100)} 1 0px`;
    css["min-width"] = "0";
  }
  if (n.kind === ControlKind.span) {
    if (!partial || has(Prop.textSize)) css["font-size"] = TEXT_SIZES[en(Prop.textSize) ?? EnumTextSize.sm] ?? TEXT_SIZES[EnumTextSize.sm];
    if (!partial || has(Prop.weight)) css["font-weight"] = WEIGHTS[en(Prop.weight) ?? EnumWeight.regular] ?? "400";
    if (p.get(Prop.italic)) css["font-style"] = "italic";
    else if (partial && has(Prop.italic)) css["font-style"] = "normal";
    if (p.get(Prop.mono)) css["font-family"] = "ui-monospace, Consolas, monospace";
    if (!partial || has(Prop.fg)) css.color = colorVar(en(Prop.fg), true) ?? "var(--pl-text)";
    const align = en(Prop.align);
    if (align === EnumTextAlign.center) css["text-align"] = "center";
    else if (align === EnumTextAlign.end) css["text-align"] = "end";
    else if (partial && has(Prop.align)) css["text-align"] = "start";
    const lines = p.get(Prop.lines);
    if (typeof lines === "number" && lines > 0) {
      css.display = "-webkit-box";
      css["-webkit-box-orient"] = "vertical";
      css["-webkit-line-clamp"] = String(Math.min(Math.trunc(lines), 1000));
      css.overflow = "hidden";
    }
    return css;
  }
  const row = en(Prop.axis) === EnumAxis.row;
  if (!partial) css.display = "flex";
  if (!partial || has(Prop.axis)) css["flex-direction"] = row ? "row" : "column";
  if (p.get(Prop.wrap)) css["flex-wrap"] = "wrap";
  else if (partial && has(Prop.wrap)) css["flex-wrap"] = "nowrap";
  for (const [prop, name] of [
    [Prop.gap, "gap"],
    [Prop.padding, "padding"],
  ]) {
    const v = p.get(prop);
    if (typeof v === "number") css[name] = units(v);
  }
  const px = p.get(Prop.paddingX);
  if (typeof px === "number") css["padding-inline"] = units(px);
  const py = p.get(Prop.paddingY);
  if (typeof py === "number") css["padding-block"] = units(py);
  if (!partial || has(Prop.crossAlign)) {
    css["align-items"] = { [EnumCrossAlign.start]: "flex-start", [EnumCrossAlign.center]: "center", [EnumCrossAlign.end]: "flex-end" }[en(Prop.crossAlign)] ?? "stretch";
  }
  if (!partial || has(Prop.justify)) {
    css["justify-content"] = { [EnumJustify.center]: "center", [EnumJustify.end]: "flex-end", [EnumJustify.between]: "space-between" }[en(Prop.justify)] ?? "flex-start";
  }
  // A size in spacing units is fixed: the element does not shrink below it.
  if (typeof p.get(Prop.width) === "number" || typeof p.get(Prop.height) === "number") css["flex-shrink"] = "0";
  for (const [u, f, name] of [
    [Prop.width, Prop.widthFraction, "width"],
    [Prop.height, Prop.heightFraction, "height"],
    [Prop.maxWidth, Prop.maxWidthFraction, "max-width"],
    [Prop.maxHeight, Prop.maxHeightFraction, "max-height"],
  ]) {
    const v = sizeOf(p, u, f);
    if (v) css[name] = v;
  }
  const bg = colorVar(en(Prop.bg));
  if (bg) css.background = bg;
  else if (partial && has(Prop.bg)) css.background = "transparent";
  const border = colorVar(en(Prop.border));
  if (border) css.border = `1px solid ${border}`;
  else if (partial && has(Prop.border)) css.border = "0";
  const radius = { [EnumRadius.sm]: "4px", [EnumRadius.md]: "8px", [EnumRadius.lg]: "12px", [EnumRadius.full]: "9999px" }[en(Prop.radius)];
  if (radius) css["border-radius"] = radius;
  else if (partial && has(Prop.radius)) css["border-radius"] = "0";
  if (n.kind === ControlKind.scroll && !partial) css[row ? "overflow-x" : "overflow-y"] = "auto";
  // UI API 1.9: an absolute box is placed in its parent primitive (every
  // primitive is `position: relative` in style.css, as in gpui).
  if (en(Prop.position) === EnumPosition.absolute) css.position = "absolute";
  else if (partial && has(Prop.position)) css.position = "relative";
  for (const [prop, name] of [
    [Prop.top, "top"],
    [Prop.left, "left"],
    [Prop.right, "right"],
    [Prop.bottom, "bottom"],
  ]) {
    const v = p.get(prop);
    if (typeof v === "number") css[name] = units(v);
  }
  return css;
}

// -- UI API 1.7: partial styles (states and width classes) ----------------------

const ENUM_STYLE_PROPS = new Set([
  Prop.position,
  Prop.axis,
  Prop.align,
  Prop.crossAlign,
  Prop.justify,
  Prop.widthFraction,
  Prop.heightFraction,
  Prop.maxWidthFraction,
  Prop.maxHeightFraction,
  Prop.bg,
  Prop.fg,
  Prop.border,
  Prop.radius,
  Prop.textSize,
  Prop.weight,
]);
const BOOL_STYLE_PROPS = new Set([Prop.wrap, Prop.italic, Prop.mono]);

/**
 * A partial style string from the compiler (`"56:11,42:4"`: prop id and
 * int value) as a props map in the wire shape (enums as `{ enum }`,
 * booleans as booleans). Malformed pairs are skipped.
 */
export function parsePartialStyle(text) {
  const props = new Map();
  for (const pair of String(text ?? "").split(",")) {
    const m = /^\s*(\d+)\s*:\s*(-?\d+(?:\.\d+)?(?:e-?\d+)?)\s*$/.exec(pair);
    if (!m) continue;
    const id = Number(m[1]);
    const v = Number(m[2]);
    props.set(id, ENUM_STYLE_PROPS.has(id) ? { enum: v } : BOOL_STYLE_PROPS.has(id) ? v !== 0 : v);
  }
  return props;
}

/** The selector of each partial style prop; `%` is the generated class. */
const PARTIAL_RULES = {
  [Prop.hover]: (c) => `.${c}:hover`,
  [Prop.active]: (c) => `.${c}:active`,
  [Prop.focus]: (c) => `.${c}:focus-visible`,
  // The width classes of style.css (SPEC.md §6.1): compact < 600px <= regular < 1200px <= wide.
  [Prop.compact]: (c) => ["@media (max-width: 599.98px)", `.${c}`],
  [Prop.regular]: (c) => ["@media (min-width: 600px) and (max-width: 1199.98px)", `.${c}`],
  [Prop.wide]: (c) => ["@media (min-width: 1200px)", `.${c}`],
};
export const PARTIAL_STYLE_PROPS = Object.keys(PARTIAL_RULES).map(Number);

/**
 * The CSS rule text of one partial style for the class `cls`. The
 * declarations are `!important`, so they win over the inline style of the
 * element (its props). Returns null for an empty style.
 */
export function partialRule(kind, propId, text, cls) {
  const css = primitiveStyle({ kind, props: parsePartialStyle(text) }, true);
  const decls = Object.entries(css).map(([k, v]) => `${k}: ${v} !important;`).join(" ");
  if (!decls) return null;
  const sel = PARTIAL_RULES[propId](cls);
  return Array.isArray(sel) ? `${sel[0]} { ${sel[1]} { ${decls} } }` : `${sel} { ${decls} }`;
}

/** The Plinth name of a browser key (UI API 1.9), the same names as the desktop (`primitives::key_name`); null for other keys. */
export function keyName(key) {
  const named = {
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
    Enter: "Enter",
    Escape: "Escape",
    " ": "Space",
    Tab: "Tab",
    Backspace: "Backspace",
    Delete: "Delete",
    Home: "Home",
    End: "End",
    PageUp: "PageUp",
    PageDown: "PageDown",
  };
  if (Object.hasOwn(named, key)) return named[key];
  return /^[A-Za-z0-9]$/.test(key) ? key.toLowerCase() : null;
}

// -- UI API 1.10: Canvas ---------------------------------------------------------

/** The shapes of a Canvas `shapes` prop (the same format as `crates/plinth-ui/src/canvas.rs`). Malformed shapes are skipped. */
export function parseShapes(text) {
  const out = [];
  for (const s of String(text ?? "").split("\u001e")) {
    if (!s) continue;
    const f = s.split("\u001f");
    const n = (i) => {
      const v = Number(f[i]);
      return f[i] !== undefined && f[i] !== "" && Number.isFinite(v) ? v : null;
    };
    const ok = (...vs) => vs.every((v) => v !== null);
    if (f[0] === "r" && ok(n(1), n(2), n(3), n(4)) && f[5] !== undefined) out.push({ kind: "rect", x: n(1), y: n(2), w: n(3), h: n(4), color: f[5] });
    else if (f[0] === "c" && ok(n(1), n(2), n(3)) && f[4] !== undefined) out.push({ kind: "circle", cx: n(1), cy: n(2), r: n(3), color: f[4] });
    else if (f[0] === "l" && ok(n(1), n(2), n(3), n(4)) && f[5] !== undefined)
      out.push({ kind: "line", x1: n(1), y1: n(2), x2: n(3), y2: n(4), color: f[5], width: n(6) ?? 1 });
    else if (f[0] === "t" && ok(n(1), n(2)) && f[3] !== undefined)
      out.push({ kind: "text", x: n(1), y: n(2), color: f[3], size: n(4) ?? 12, text: f.slice(5).join("\u001f") });
  }
  return out;
}

/** A color token name of a shape ("accent", "text.muted") as CSS, or null. */
export function shapeColor(name) {
  const key = String(name).replace(/\.([a-z])/g, (_, c) => c.toUpperCase());
  return Object.hasOwn(EnumColor, key) ? colorVar(EnumColor[key], name === "accent") : null;
}

/** One generated class for each distinct partial style, in one style element of the document. */
class PartialStyles {
  constructor(doc) {
    this.doc = doc;
    this.classes = new Map(); // "kind|prop|text" -> class name
    this.sheet = null;
  }

  classFor(kind, propId, text) {
    const key = `${kind}|${propId}|${text}`;
    if (this.classes.has(key)) return this.classes.get(key);
    const cls = `pl-st-${this.classes.size + 1}`;
    const rule = partialRule(kind, propId, text, cls);
    if (rule) {
      if (!this.sheet) {
        const style = this.doc.createElement("style");
        style.id = "pl-partial-styles";
        this.doc.head.append(style);
        this.sheet = style.sheet;
      }
      this.sheet.insertRule(rule, this.sheet.cssRules.length);
    }
    this.classes.set(key, cls);
    return cls;
  }
}

/** Applies `css` to `e`, changing only the properties that differ from `last` (the previous call). */
function applyStyle(e, css, last) {
  for (const name of Object.keys(last)) if (!(name in css)) e.style.removeProperty(name);
  for (const [name, value] of Object.entries(css)) if (last[name] !== value) e.style.setProperty(name, value);
  return css;
}

export class Tree {
  constructor() {
    this.nodes = new Map();
    this.roots = new Map(); // screen id -> root node id
    this.primaryScreens = []; // [{screen}] in mark-primary order
    this.currentScreen = 0;
    this.stack = []; // push/pop navigation (within a screen)
    this.onChange = () => {};
    this.takeChanges();
  }

  node(id) {
    return this.nodes.get(id);
  }

  /** The changes since the last call, and a fresh, empty record. */
  takeChanges() {
    const changes = this.changes;
    this.changes = {
      dirty: new Set(), // nodes whose props, text or listeners changed
      children: new Set(), // nodes whose children changed (insert, move, remove)
      removed: new Set(), // removed nodes, with their subtrees
      created: new Set(),
      nav: false, // roots, primary screens or the stack changed
    };
    return changes;
  }

  apply(ops) {
    for (const op of ops) this.applyOne(op);
    this.onChange();
  }

  /** Removes `id` from its parent's children (if it has a parent). */
  detach(id) {
    const n = this.nodes.get(id);
    if (!n || !n.parent) return;
    const p = this.nodes.get(n.parent);
    if (p) {
      const i = p.children.indexOf(id);
      if (i >= 0) p.children.splice(i, 1);
      this.changes.children.add(p.id);
    }
    n.parent = 0;
  }

  applyOne(op) {
    const ch = this.changes;
    switch (op.op) {
      case "create":
        this.nodes.set(op.id, new Node(op.id, op.kind));
        ch.created.add(op.id);
        ch.dirty.add(op.id);
        break;
      case "remove": {
        // SPEC.md §8.2: removes the subtree, and the host frees its ids.
        if (!this.nodes.has(op.id)) break;
        this.detach(op.id);
        const stack = [op.id];
        while (stack.length) {
          const id = stack.pop();
          const n = this.nodes.get(id);
          if (!n) continue;
          for (const c of n.children) {
            if (this.nodes.get(c)?.parent === id) stack.push(c);
          }
          this.nodes.delete(id);
          ch.removed.add(id);
          ch.created.delete(id);
        }
        for (const [screen, root] of this.roots) {
          if (root === op.id) {
            this.roots.delete(screen);
            ch.nav = true;
          }
        }
        break;
      }
      case "insert":
      case "move": {
        const parent = this.nodes.get(op.parent);
        const child = this.nodes.get(op.id);
        if (!parent || !child || op.id === op.parent) break;
        if (child.parent === op.parent) {
          const i = parent.children.indexOf(op.id);
          if (i >= 0) parent.children.splice(i, 1);
        } else {
          this.detach(op.id);
        }
        // `before` 0 means append; an unknown anchor appends too.
        const idx = op.before ? parent.children.indexOf(op.before) : -1;
        if (idx === -1) parent.children.push(op.id);
        else parent.children.splice(idx, 0, op.id);
        child.parent = op.parent;
        ch.children.add(op.parent);
        break;
      }
      case "set-prop": {
        const n = this.nodes.get(op.id);
        if (!n) break;
        n.props.set(op.prop, op.value);
        ch.dirty.add(op.id);
        // A screen title is also the nav item's text and the page title.
        if (op.prop === Prop.title && n.kind === ControlKind.screen) ch.nav = true;
        break;
      }
      case "text": {
        const n = this.nodes.get(op.id);
        if (n) {
          n.text = op.value;
          ch.dirty.add(op.id);
        }
        break;
      }
      case "listen": {
        const n = this.nodes.get(op.id);
        if (n) {
          n.listeners.set(op.event, op.handler);
          ch.dirty.add(op.id);
        }
        break;
      }
      case "unlisten": {
        const n = this.nodes.get(op.id);
        if (n) {
          n.listeners.delete(op.event);
          ch.dirty.add(op.id);
        }
        break;
      }
      case "set-root":
        if (op.id) this.roots.set(op.screen, op.id);
        else this.roots.delete(op.screen);
        ch.nav = true;
        break;
      case "navigate":
        this.applyNavigate(op);
        ch.nav = true;
        break;
      case "snapshot":
        break; // dev-only hot reload, not relevant to the web host
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

// -- DOM helpers -------------------------------------------------------------

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

/** Sets the text of an element only when it differs. A write of the same
 * text would replace the text node and end a text selection in it. */
function setText(e, s) {
  s = s ?? "";
  if (e.textContent !== s) e.textContent = s;
}

/** Sets (or, for null/undefined/false, removes) an attribute when it differs. */
function setAttr(e, name, value) {
  if (value === null || value === undefined || value === false) {
    if (e.hasAttribute(name)) e.removeAttribute(name);
  } else {
    const s = String(value);
    if (e.getAttribute(name) !== s) e.setAttribute(name, s);
  }
}

/** Sets a DOM property only when it differs. */
function setProp(e, name, value) {
  if (e[name] !== value) e[name] = value;
}

/** A label-like element: its text, hidden when there is no text. */
function setOptionalText(e, s) {
  setText(e, s ?? "");
  setProp(e, "hidden", !s);
}

/**
 * The longest increasing subsequence of `seq` (numbers), as a Set of the
 * values in it. `reconcile` keeps these elements in place and moves only
 * the others, so a keyed reorder makes the fewest DOM moves.
 */
function lisValues(seq) {
  const tails = []; // index into seq of the smallest tail of each length
  const prev = new Array(seq.length);
  for (let i = 0; i < seq.length; i++) {
    let lo = 0, hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (seq[tails[mid]] < seq[i]) lo = mid + 1;
      else hi = mid;
    }
    prev[i] = lo > 0 ? tails[lo - 1] : -1;
    tails[lo] = i;
  }
  const out = new Set();
  for (let i = tails.length ? tails[tails.length - 1] : -1; i >= 0; i = prev[i]) out.add(seq[i]);
  return out;
}

/**
 * Makes the node elements in `slot` equal to `desired`, in order, with the
 * fewest DOM moves. Other children of `slot` (fixed parts of a control,
 * such as a Dialog title) must come before them. The slot keeps the last
 * list (`_plKids`), so the equal start and end of the two lists cost no DOM
 * access: one changed row of 10,000 touches only that row.
 */
function reconcile(slot, desired) {
  const old = slot._plKids ?? [];
  slot._plKids = desired;
  let start = 0, oldEnd = old.length, newEnd = desired.length;
  while (start < oldEnd && start < newEnd && old[start] === desired[start]) start++;
  while (oldEnd > start && newEnd > start && old[oldEnd - 1] === desired[newEnd - 1]) {
    oldEnd--;
    newEnd--;
  }
  if (start === oldEnd && start === newEnd) return;
  const mid = desired.slice(start, newEnd);
  const pos = new Map(mid.map((e, i) => [e, i]));
  const present = []; // positions in `mid` of the old elements that stay
  for (let i = start; i < oldEnd; i++) {
    const e = old[i];
    if (e.parentNode !== slot) continue; // moved to another parent, or made again
    if (pos.has(e)) present.push(pos.get(e));
    else slot.removeChild(e);
  }
  const keep = lisValues(present);
  let anchor = newEnd < desired.length ? desired[newEnd] : null;
  for (let i = mid.length - 1; i >= 0; i--) {
    if (!keep.has(i)) slot.insertBefore(mid[i], anchor);
    anchor = mid[i];
  }
}

// -- Chart data --------------------------------------------------------------

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

let chartIds = 0;

// -- The renderer ------------------------------------------------------------

/**
 * A view is `{ id, kind, el, update(n), place?(children), rebuild?(n) }`:
 * `el` is the node's element (tagged with `data-pl-id`),
 * `update` writes the node's props into the existing element (it compares
 * before it writes, so it can run any number of times), `place` puts the
 * child views' elements into the control (the default: all of them, in
 * order, into `slot`), and `rebuild` tells when the element must be made
 * again.
 */
export class DomRenderer {
  /** `assets` is a `Map<path, Uint8Array>` from `readPlnt` (SPEC.md §10.1, `<Image>`). */
  constructor(tree, container, app, assets = new Map()) {
    this.tree = tree;
    this.container = container;
    this.app = app;
    this.assets = assets;
    this.assetUrls = new Map(); // path -> blob: URL, built lazily and kept for the page's life
    this.views = new Map(); // node id -> view
    this.nav = el("nav", "pl-nav", { "aria-label": "Screens" });
    this.navButtons = [];
    this.main = el("main", "pl-screen");
    this.shown = null; // the root view in `main`
    tree.onChange = () => this.render();
  }

  assetUrl(path) {
    if (this.assetUrls.has(path)) return this.assetUrls.get(path);
    const bytes = this.assets.get(path);
    if (!bytes) return null;
    const url = URL.createObjectURL(new Blob([bytes], { type: assetMimeType(path) }));
    this.assetUrls.set(path, url);
    return url;
  }

  node(id) {
    return this.tree.node(id);
  }

  send(handler, eventCode, value) {
    this.app.onEvent({ kind: "ui", handler, event: eventCode, value });
  }

  /** Sends `eventCode` to the node's listener for it, if there is one. */
  fire(id, eventCode, value = null) {
    const handler = this.node(id)?.listeners.get(eventCode);
    if (handler !== undefined) this.send(handler, eventCode, value);
  }

  /**
   * A `change` of a two-way bound `value` (SPEC.md §8.4). The guest does not
   * echo the value back, so the host keeps it in the tree, as the desktop
   * host does (`Tree::set_local_prop`).
   */
  sendChange(id, value) {
    const n = this.node(id);
    if (!n) return;
    n.props.set(Prop.value, value);
    const handler = n.listeners.get(Event.change);
    if (handler !== undefined) this.send(handler, Event.change, value);
  }

  /** Applies the tree's changes since the last render to the DOM. */
  render() {
    const ch = this.tree.takeChanges();
    for (const id of ch.removed) this.views.delete(id);
    for (const id of ch.created) this.views.delete(id); // an id used again
    for (const id of ch.dirty) {
      const v = this.views.get(id);
      if (v) this.updateView(v);
    }
    for (const id of ch.children) {
      const v = this.views.get(id);
      if (v) this.placeChildren(v);
    }
    this.renderShell(ch.nav);
  }

  /** Local navigation (nav bar, back button): no op from the guest. */
  navigateLocal(change) {
    change();
    this.tree.changes.nav = true;
    this.render();
  }

  /** The view of node `id`; builds it (and its subtree) the first time. */
  viewFor(id) {
    const existing = this.views.get(id);
    if (existing) return existing;
    const n = this.node(id);
    return n ? this.build(n) : null;
  }

  build(n) {
    const v = this.makeView(n);
    v.id = n.id;
    v.kind = n.kind;
    v.el.dataset.plId = String(n.id);
    this.views.set(n.id, v);
    v.update(n);
    this.placeChildren(v);
    return v;
  }

  updateView(v) {
    const n = this.node(v.id);
    if (!n) return;
    if (v.kind !== n.kind || v.rebuild?.(n)) {
      this.views.delete(v.id);
      const fresh = this.build(n);
      v.el.replaceWith(fresh.el);
      if (this.shown === v) this.shown = fresh;
      return;
    }
    v.update(n);
  }

  placeChildren(v) {
    const n = this.node(v.id);
    if (!n) return;
    if (!v.place && !v.slot) return;
    const children = [];
    for (const cid of n.children) {
      const c = this.viewFor(cid);
      if (c) children.push(c);
    }
    if (v.place) v.place(children);
    else reconcile(v.slot, children.map((c) => c.el));
    // A List with no rows (only an EmptyState, or nothing) is not a list
    // for assistive technology: role="list" needs listitem children.
    if (v.kind === ControlKind.list) {
      const items = children.some((c) => c.el.getAttribute("role") === "listitem");
      if (items !== v.el.hasAttribute("role")) {
        if (items) v.el.setAttribute("role", "list");
        else v.el.removeAttribute("role");
      }
    }
  }

  /** The nav bar, the shown screen and the page title. */
  renderShell(navChanged) {
    const t = this.tree;
    if (this.main.parentNode !== this.container) {
      this.container.replaceChildren(this.nav, this.main);
    }
    if (navChanged) {
      this.nav.hidden = t.primaryScreens.length === 0;
      t.primaryScreens.forEach((p, i) => {
        let btn = this.navButtons[i];
        if (!btn) {
          btn = el("button", "pl-nav-item");
          btn.addEventListener("click", () => {
            const screen = t.primaryScreens[i]?.screen;
            if (screen === undefined) return;
            this.navigateLocal(() => {
              t.currentScreen = screen;
              t.stack = [];
            });
          });
          this.navButtons.push(btn);
          this.nav.appendChild(btn);
        }
        // The nav shows each screen's title, as the desktop shell does.
        const root = t.node(t.roots.get(p.screen));
        setText(btn, root?.props.get(Prop.title) ?? `Screen ${p.screen}`);
        const current = p.screen === t.currentScreen;
        setProp(btn, "disabled", current);
        setAttr(btn, "aria-current", current ? "page" : null);
      });
      while (this.navButtons.length > t.primaryScreens.length) this.navButtons.pop().remove();
    }

    const rootId = t.roots.get(t.topScreen());
    const view = rootId !== undefined ? this.viewFor(rootId) : null;
    if (view !== this.shown || (view && view.el.parentNode !== this.main)) {
      this.main.replaceChildren(...(view ? [view.el] : []));
      this.shown = view;
      // A Sheet or Dialog can open only in the document: open the ones of
      // a screen that comes back.
      if (view) for (const d of view.el.querySelectorAll("dialog")) d._plSync?.();
    }
    if (navChanged && view) view.update(this.node(rootId)); // the back button
    if (navChanged) {
      // document.title follows the current screen, matching the desktop
      // window title (SPEC.md §6).
      const screenTitle = view ? this.node(rootId)?.props.get(Prop.title) : null;
      const title = screenTitle ? `${screenTitle} — Plinth` : "Plinth web host";
      if (document.title !== title) document.title = title;
    }
  }

  makeView(n) {
    const kname = kindName[n.kind] ?? `kind${n.kind}`;
    switch (kname) {
      case "screen": return this.screenView();
      case "section": return this.sectionView();
      case "text": return this.textView();
      case "heading": return this.headingView(n);
      case "button": return this.buttonView();
      case "textField": return this.textFieldView();
      case "textArea": return this.textAreaView();
      case "toggle": return this.checkView("pl-toggle", "pl-toggle-label");
      case "checkbox": return this.checkView("pl-checkbox-field", "pl-checkbox-label");
      case "list": return this.containerView(el("div", "pl-list", { role: "list" }));
      case "row": return this.rowView();
      case "group": return this.containerView(el("div", "pl-group"));
      case "empty": return this.emptyView();
      case "slider": return this.sliderView();
      case "numberField": return this.numberFieldView();
      case "picker": return this.pickerView();
      case "progress": return this.progressView();
      case "badge": return this.badgeView();
      case "tabs": return this.tabsView();
      case "sheet":
      case "dialog": return this.overlayView(kname);
      case "menu": return this.menuView();
      // Not every Grid child is a Row (`role=listitem`), so the grid itself
      // cannot always take `role=list`.
      case "grid": return this.containerView(el("div", "pl-grid"));
      case "action": return this.actionView();
      case "image": return this.imageView();
      case "icon": return this.iconView();
      case "datePicker": return this.datePickerView();
      case "chart": return this.chartView();
      case "box":
      case "scroll":
      case "pressable": return this.primitiveBoxView(kname);
      case "span": return this.spanView();
      case "canvas": return this.canvasView();
      default: return this.containerView(el("div", `pl-${kname}`));
    }
  }

  /**
   * A Level 2 box (`Box`, `Scroll`, `Pressable`): a `<div>` with inline
   * flex styles. A Pressable is a tab stop with the role button or link;
   * Enter and Space press it (Space only for a button, as in HTML). A
   * Scroll is a tab stop too, so the keyboard can scroll it.
   */
  /** The generated classes of the partial styles of `n` (states and width classes) on `e`; returns them. */
  partialClasses(e, n, previous) {
    this.partialStyles ??= new PartialStyles(e.ownerDocument);
    const now = [];
    for (const propId of PARTIAL_STYLE_PROPS) {
      const text = n.props.get(propId);
      if (typeof text === "string" && text) now.push(this.partialStyles.classFor(n.kind, propId, text));
    }
    for (const c of previous) if (!now.includes(c)) e.classList.remove(c);
    for (const c of now) if (!previous.includes(c)) e.classList.add(c);
    return now;
  }

  /**
   * UI API 1.12: the pointer events of the node `getId()` on `e`. `x` and `y`
   * are from the top-left corner of `e`, in units of `pxPerUnit(rect)` pixels.
   * A pointer-down captures the pointer, so the element also gets the moves
   * outside it and the pointer-up (a drag), as on the desktop.
   */
  pointerEvents(e, getId, pxPerUnit) {
    const has = (code) => this.node(getId())?.listeners.get(code) !== undefined;
    const send = (code, ev) => {
      if (!has(code)) return;
      const r = e.getBoundingClientRect();
      const k = Math.max(pxPerUnit(r), Number.EPSILON);
      this.fire(getId(), code, [(ev.clientX - r.left) / k, (ev.clientY - r.top) / k]);
    };
    e.addEventListener("pointerdown", (ev) => {
      if (ev.button !== 0 || !(has(Event.pointerDown) || has(Event.pointerMove) || has(Event.pointerUp))) return;
      try {
        e.setPointerCapture(ev.pointerId);
      } catch {
        // A synthetic event has no active pointer to capture.
      }
      send(Event.pointerDown, ev);
    });
    e.addEventListener("pointermove", (ev) => send(Event.pointerMove, ev));
    e.addEventListener("pointerup", (ev) => {
      if (ev.button === 0) send(Event.pointerUp, ev);
    });
  }

  /** A box with a pointer-move handler takes touch moves itself (no page scroll on a drag). */
  static touchAction(n) {
    return n.listeners.get(Event.pointerMove) !== undefined ? "none" : null;
  }

  primitiveBoxView(kname) {
    const e = el("div", `pl-${kname}`);
    let id = 0;
    let last = {};
    let partials = [];
    let disabled = false;
    let link = false;
    // UI API 1.9: onKeyDown/onKeyUp. A held key sends no repeats.
    const key = (eventCode) => (ev) => {
      if (eventCode === Event.keyDown && ev.repeat) return;
      const name = keyName(ev.key);
      if (name === null || this.node(id)?.listeners.get(eventCode) === undefined) return;
      // The app handles the key: no page scroll, no Space press of a button.
      if (name.startsWith("Arrow") || name === "Space" || name.startsWith("Page") || name === "Home" || name === "End") ev.preventDefault();
      ev.stopPropagation();
      this.fire(id, eventCode, name);
    };
    e.addEventListener("keydown", key(Event.keyDown));
    e.addEventListener("keyup", key(Event.keyUp));
    this.pointerEvents(e, () => id, () => UNIT);
    if (kname === "pressable") {
      const press = () => {
        if (!disabled) this.fire(id, Event.press);
      };
      e.addEventListener("click", press);
      e.addEventListener("keydown", (ev) => {
        if (ev.target !== e) return;
        if (ev.key === "Enter" || (ev.key === " " && !link)) {
          ev.preventDefault();
          press();
        }
      });
    }
    return {
      el: e,
      slot: e,
      update: (n) => {
        id = n.id;
        const css = primitiveStyle(n);
        const touch = DomRenderer.touchAction(n);
        if (touch) css["touch-action"] = touch;
        last = applyStyle(e, css, last);
        partials = this.partialClasses(e, n, partials);
        const label = n.props.get(Prop.label) ?? null;
        if (kname === "pressable") {
          link = n.props.get(Prop.role)?.enum === EnumPressableRole.link;
          disabled = !!n.props.get(Prop.disabled);
          setAttr(e, "role", link ? "link" : "button");
          setAttr(e, "aria-label", label);
          setAttr(e, "aria-disabled", disabled ? "true" : null);
          setAttr(e, "tabindex", disabled ? null : "0");
          // The runtime's hover, unless the app gives a background or its own hover.
          e.classList.toggle("pl-pressable-plain", !colorVar(n.props.get(Prop.bg)?.enum) && !n.props.get(Prop.hover));
          e.classList.toggle("pl-pressable-own-hover", !!n.props.get(Prop.hover));
        } else {
          const keys = n.listeners.get(Event.keyDown) !== undefined || n.listeners.get(Event.keyUp) !== undefined;
          setAttr(e, "role", label ? "group" : null);
          setAttr(e, "aria-label", label);
          setAttr(e, "tabindex", kname === "scroll" || keys ? "0" : null);
        }
      },
    };
  }

  /**
   * A Canvas (UI API 1.10): inline SVG with a viewBox, so the drawing keeps
   * its aspect ratio and scales to the width. The shapes are drawn again
   * only when the `shapes` prop changes.
   */
  canvasView() {
    const svg = svgEl("svg", { class: "pl-canvas", role: "img", preserveAspectRatio: "xMidYMid meet" });
    let lastShapes = null;
    let last = {};
    let id = 0;
    let viewWidth = 100;
    // Pointer positions in view units: the canvas scales the view to its width.
    this.pointerEvents(svg, () => id, (r) => r.width / viewWidth);
    return {
      el: svg,
      update: (n) => {
        id = n.id;
        const vw = Math.max(1, n.props.get(Prop.viewWidth) ?? 100);
        viewWidth = vw;
        const vh = Math.max(1, n.props.get(Prop.viewHeight) ?? 100);
        setAttr(svg, "viewBox", `0 0 ${vw} ${vh}`);
        setAttr(svg, "aria-label", n.props.get(Prop.label) ?? null);
        const css = {};
        const w = sizeOf(n.props, Prop.width, Prop.widthFraction);
        css.width = w ?? "100%";
        const mw = sizeOf(n.props, Prop.maxWidth, Prop.maxWidthFraction);
        if (mw) css["max-width"] = mw;
        if (w) css["flex-shrink"] = "0";
        const grow = n.props.get(Prop.grow);
        if (typeof grow === "number" && grow > 0) css.flex = `${Math.min(Math.trunc(grow), 100)} 1 0px`;
        css["aspect-ratio"] = `${vw} / ${vh}`;
        const touch = DomRenderer.touchAction(n);
        if (touch) css["touch-action"] = touch;
        last = applyStyle(svg, css, last);
        const text = n.props.get(Prop.shapes) ?? "";
        if (text === lastShapes) return;
        lastShapes = text;
        const els = parseShapes(text).map((s) => {
          const color = shapeColor(s.color) ?? "currentColor";
          switch (s.kind) {
            case "rect":
              return svgEl("rect", { x: s.x, y: s.y, width: Math.max(0, s.w), height: Math.max(0, s.h), style: `fill: ${color}` });
            case "circle":
              return svgEl("circle", { cx: s.cx, cy: s.cy, r: Math.max(0, s.r), style: `fill: ${color}` });
            case "line":
              return svgEl("line", { x1: s.x1, y1: s.y1, x2: s.x2, y2: s.y2, "stroke-linecap": "round", style: `stroke: ${color}; stroke-width: ${s.width}` });
            default: {
              const t = svgEl("text", { x: s.x, y: s.y, "font-size": s.size, style: `fill: ${color}` });
              t.textContent = s.text;
              return t;
            }
          }
        });
        svg.replaceChildren(...els);
      },
    };
  }

  /** A Level 2 Span: styled text. */
  spanView() {
    const e = el("span", "pl-span");
    let last = {};
    let partials = [];
    return {
      el: e,
      update: (n) => {
        last = applyStyle(e, primitiveStyle(n), last);
        partials = this.partialClasses(e, n, partials);
        setText(e, n.text ?? "");
      },
    };
  }

  /** A plain element whose children are the node's children. */
  containerView(e) {
    return { el: e, slot: e, update() {} };
  }

  /** A `<label class="pl-field">` with its label text and `control`. */
  field(control, className = "pl-field", tag = "label") {
    const wrap = el(tag, className);
    const label = wrap.appendChild(el("span", "pl-field-label"));
    wrap.appendChild(control);
    return { wrap, label };
  }

  screenView() {
    const section = el("section", "pl-screen-body");
    const header = section.appendChild(el("div", "pl-screen-header"));
    const back = header.appendChild(el("button", "pl-back", { "aria-label": "Back" }));
    back.textContent = "← Back";
    back.addEventListener("click", () => this.navigateLocal(() => this.tree.stack.pop()));
    const title = header.appendChild(el("h1", "pl-title"));
    // Actions become `action`-kind child nodes (SPEC.md §6.3 UI API 1.2); they
    // go in a toolbar above the other children rather than inline.
    const toolbar = section.appendChild(el("div", "pl-toolbar"));
    return {
      el: section,
      update: (n) => {
        setProp(back, "hidden", this.tree.stack.length === 0);
        setOptionalText(title, n.props.get(Prop.title));
      },
      place: (children) => {
        const actions = children.filter((c) => c.kind === ControlKind.action);
        reconcile(toolbar, actions.map((c) => c.el));
        setProp(toolbar, "hidden", actions.length === 0);
        reconcile(section, children.filter((c) => c.kind !== ControlKind.action).map((c) => c.el));
      },
    };
  }

  /** An `Action`: a button in a toolbar (Screen), a Menu, a Dialog, or alone. */
  actionView() {
    const b = el("button", "pl-action");
    let id = 0;
    b.addEventListener("click", () => {
      const n = this.node(id);
      if (!n || n.listeners.get(Event.press) === undefined) return;
      const destructive = n.props.get(Prop.role)?.enum === 2;
      const needsConfirm = destructive && n.props.get(Prop.confirm) !== false;
      if (needsConfirm && !window.confirm(`${b.textContent}?`)) return;
      this.fire(id, Event.press);
    });
    return {
      el: b,
      update: (n) => {
        id = n.id;
        setText(b, n.props.get(Prop.label) ?? "");
        b.classList.toggle("pl-action-destructive", n.props.get(Prop.role)?.enum === 2);
      },
    };
  }

  sectionView() {
    const fieldset = el("fieldset", "pl-section");
    const legend = fieldset.appendChild(el("legend", "pl-section-title"));
    return {
      el: fieldset,
      slot: fieldset,
      update: (n) => setOptionalText(legend, n.props.get(Prop.title)),
    };
  }

  textView() {
    const p = el("p", "pl-text");
    return { el: p, update: (n) => setText(p, n.text) };
  }

  headingView(n) {
    const tagOf = (node) => `H${Math.min(Math.max(node.props.get(Prop.level) || 2, 1), 6)}`;
    const h = el(tagOf(n).toLowerCase(), "pl-heading");
    return {
      el: h,
      rebuild: (node) => tagOf(node) !== h.tagName,
      update: (node) => setText(h, node.text ?? node.props.get(Prop.title)),
    };
  }

  buttonView() {
    const b = el("button", "pl-button");
    let id = 0;
    b.addEventListener("click", () => this.fire(id, Event.press));
    return {
      el: b,
      update: (n) => {
        id = n.id;
        setText(b, n.props.get(Prop.label) ?? n.text ?? "");
        setProp(b, "disabled", !!n.props.get(Prop.disabled));
      },
    };
  }

  textFieldView() {
    const input = el("input", "pl-input", { type: "text" });
    // `error` (SPEC.md §6.3): a message under the input. It is outside the
    // label's name (aria-hidden) and is the input's description instead.
    const col = el("span", "pl-input-col");
    col.appendChild(input);
    const error = col.appendChild(el("span", "pl-field-error", { "aria-hidden": "true" }));
    const { wrap, label } = this.field(col);
    let id = 0;
    input.addEventListener("input", () => this.sendChange(id, input.value));
    input.addEventListener("keydown", (e) => {
      // Enter also ends an IME composition; that Enter is not a submit.
      if (e.key === "Enter" && !e.isComposing) this.fire(id, Event.submit, input.value);
    });
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setOptionalText(label, n.props.get(Prop.label));
        setProp(input, "value", n.props.get(Prop.value) ?? "");
        setProp(input, "placeholder", n.props.get(Prop.placeholder) ?? "");
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
        // An empty `error` means no error, as on the desktop.
        const message = n.props.get(Prop.error) || "";
        setOptionalText(error, message);
        error.id = `pl-error-${n.id}`;
        setAttr(input, "aria-invalid", message ? "true" : null);
        setAttr(input, "aria-describedby", message ? error.id : null);
      },
    };
  }

  textAreaView() {
    const input = el("textarea", "pl-textarea");
    const { wrap, label } = this.field(input);
    let id = 0;
    input.addEventListener("input", () => this.sendChange(id, input.value));
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setOptionalText(label, n.props.get(Prop.label));
        setProp(input, "value", n.props.get(Prop.value) ?? "");
        setProp(input, "placeholder", n.props.get(Prop.placeholder) ?? "");
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
      },
    };
  }

  /** Toggle and Checkbox: a native checkbox in a label. */
  checkView(wrapClass, labelClass) {
    const wrap = el("label", wrapClass);
    const input = wrap.appendChild(el("input", "pl-checkbox", { type: "checkbox" }));
    const label = wrap.appendChild(el("span", labelClass));
    let id = 0;
    input.addEventListener("change", () => this.sendChange(id, input.checked));
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setProp(input, "checked", !!n.props.get(Prop.value));
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
        setOptionalText(label, n.props.get(Prop.label));
      },
    };
  }

  rowView() {
    const row = el("div", "pl-row", { role: "listitem" });
    const header = row.appendChild(el("div", "pl-row-header"));
    const title = header.appendChild(el("span", "pl-row-title"));
    const subtitle = header.appendChild(el("span", "pl-row-subtitle"));
    const trailing = row.appendChild(el("span", "pl-row-trailing"));
    const actions = row.appendChild(el("div", "pl-row-actions"));
    let id = 0;
    const pressable = () => this.node(id)?.listeners.get(Event.press) !== undefined;
    // A pressable Row behaves like a button for keyboard users (Tab to
    // focus, Enter/Space to activate), matching the desktop renderer. A
    // press on a control inside the row (a Toggle) is that control's.
    row.addEventListener("click", (e) => {
      if (pressable() && !actions.contains(e.target)) this.fire(id, Event.press);
    });
    row.addEventListener("keydown", (e) => {
      if (e.target !== row || !pressable() || (e.key !== "Enter" && e.key !== " ")) return;
      e.preventDefault();
      this.fire(id, Event.press);
    });
    return {
      el: row,
      slot: actions,
      update: (n) => {
        id = n.id;
        setOptionalText(title, n.props.get(Prop.title));
        setOptionalText(subtitle, n.props.get(Prop.subtitle));
        setOptionalText(trailing, n.props.get(Prop.trailing));
        const canPress = n.listeners.get(Event.press) !== undefined;
        row.classList.toggle("pl-row-pressable", canPress);
        setAttr(row, "tabindex", canPress ? "0" : null);
        // The selected row of its list (UI API 1.8). `aria-selected` is not
        // allowed on a listitem; `aria-current` is.
        const selected = !!n.props.get(Prop.selected);
        row.classList.toggle("pl-row-selected", selected);
        setAttr(row, "aria-current", selected ? "true" : null);
      },
    };
  }

  emptyView() {
    const wrap = el("div", "pl-empty");
    const title = wrap.appendChild(el("p", "pl-empty-title"));
    const message = wrap.appendChild(el("p", "pl-empty-message"));
    return {
      el: wrap,
      update: (n) => {
        setOptionalText(title, n.props.get(Prop.title));
        setOptionalText(message, n.props.get(Prop.message));
      },
    };
  }

  sliderView() {
    const input = el("input", "pl-slider", { type: "range" });
    const { wrap, label } = this.field(input, "pl-field pl-slider-field");
    let id = 0;
    // Pointer drag on the track is native <input type=range> behavior,
    // snapped to `step` by the browser (SPEC.md §6.3).
    input.addEventListener("input", () => this.sendChange(id, Number(input.value)));
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setOptionalText(label, n.props.get(Prop.label));
        setAttr(input, "min", n.props.get(Prop.min) ?? 0);
        setAttr(input, "max", n.props.get(Prop.max) ?? 100);
        setAttr(input, "step", n.props.get(Prop.step));
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
        const v = Number(n.props.get(Prop.value) ?? 0);
        if (Number(input.value) !== v) input.value = String(v);
      },
    };
  }

  numberFieldView() {
    const input = el("input", "pl-input", { type: "number" });
    const { wrap, label } = this.field(input);
    let id = 0;
    input.addEventListener("input", () => {
      const v = input.valueAsNumber;
      if (!Number.isNaN(v)) this.sendChange(id, v);
    });
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setOptionalText(label, n.props.get(Prop.label));
        setAttr(input, "min", n.props.get(Prop.min));
        setAttr(input, "max", n.props.get(Prop.max));
        setAttr(input, "step", n.props.get(Prop.step));
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
        // Compare numbers: "1." and "1.0" while the user types equal 1.
        const v = Number(n.props.get(Prop.value) ?? 0);
        if (input.valueAsNumber !== v) input.value = String(v);
      },
    };
  }

  /** `Picker.options` arrive joined with U+001F (SPEC.md §6.3 UI API 1.2):
   * a segmented control for four options or less, else a `<select>`. */
  pickerView() {
    const wrap = el("div", "pl-field pl-picker-field");
    const label = wrap.appendChild(el("span", "pl-field-label"));
    let id = 0;
    let optionsKey = null;
    let inputs = []; // [option, element] pairs
    let control = null;
    const build = (options) => {
      control?.remove();
      inputs = [];
      if (options.length <= 4) {
        control = el("div", "pl-segmented", { role: "radiogroup" });
        for (const opt of options) {
          const optLabel = control.appendChild(el("label", "pl-segment"));
          const radio = optLabel.appendChild(el("input", "pl-segment-input", { type: "radio", name: `picker-${id}` }));
          radio.addEventListener("change", () => this.sendChange(id, opt));
          optLabel.appendChild(el("span", "pl-segment-label")).textContent = opt;
          inputs.push([opt, radio]);
        }
      } else {
        control = el("select", "pl-select");
        for (const opt of options) {
          const o = control.appendChild(el("option"));
          o.value = opt;
          o.textContent = opt;
          inputs.push([opt, o]);
        }
        control.addEventListener("change", () => this.sendChange(id, control.value));
      }
      wrap.appendChild(control);
    };
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        const labelText = n.props.get(Prop.label);
        const key = n.props.get(Prop.options) ?? "";
        if (key !== optionsKey) {
          optionsKey = key;
          build(key.split("\u001f").filter((s) => s.length > 0));
        }
        setOptionalText(label, labelText);
        setAttr(control, "aria-label", labelText ?? "");
        const current = n.props.get(Prop.value) ?? "";
        for (const [opt, e] of inputs) setProp(e, e.tagName === "OPTION" ? "selected" : "checked", opt === current);
      },
    };
  }

  progressView() {
    const progress = el("progress", "pl-progress");
    const { wrap, label } = this.field(progress, "pl-progress-field", "div");
    return {
      el: wrap,
      update: (n) => {
        const text = n.props.get(Prop.label);
        setOptionalText(label, text);
        setAttr(progress, "aria-label", text);
        const value = n.props.get(Prop.value);
        // No value: an indeterminate bar.
        setAttr(progress, "value", value);
        setAttr(progress, "max", value == null ? null : "1");
      },
    };
  }

  badgeView() {
    const b = el("span", "pl-badge");
    return { el: b, update: (n) => setText(b, n.props.get(Prop.label) ?? n.props.get(Prop.title) ?? "") };
  }

  /** `<Icon>` (SPEC.md §6.3, UI API 1.4): a glyph from the runtime icon
   * set. Decorative by default (hidden from screen readers); `label`
   * gives it an accessible name. */
  iconView() {
    const span = el("span", "pl-icon");
    return {
      el: span,
      update: (n) => {
        const label = n.props.get(Prop.label);
        const tone = n.props.get(Prop.tone)?.enum;
        setProp(span, "className", ["pl-icon", TONE_CLASS[tone]].filter(Boolean).join(" "));
        setText(span, ICON_GLYPH[n.props.get(Prop.icon) ?? ""] ?? "•");
        setAttr(span, "role", label ? "img" : null);
        setAttr(span, "aria-label", label || null);
        setAttr(span, "aria-hidden", label ? null : "true");
      },
    };
  }

  /** `<DatePicker>` (SPEC.md §6.3, UI API 1.4): a native date/time input,
   * two-way bound exactly like `TextField`. */
  datePickerView() {
    const input = el("input", "pl-input", { type: "date" });
    const { wrap, label } = this.field(input);
    let id = 0;
    input.addEventListener("change", () => this.sendChange(id, input.value));
    return {
      el: wrap,
      update: (n) => {
        id = n.id;
        setOptionalText(label, n.props.get(Prop.label));
        const mode = n.props.get(Prop.mode)?.enum ?? EnumDatePickerMode.date;
        const type = mode === EnumDatePickerMode.time ? "time" : mode === EnumDatePickerMode.datetime ? "datetime-local" : "date";
        if (input.type !== type) input.type = type;
        setProp(input, "value", n.props.get(Prop.value) ?? "");
        setProp(input, "disabled", !!n.props.get(Prop.disabled));
      },
    };
  }

  /** `<Image>` (SPEC.md §6.3, §10.1): an asset from the package. Sized by
   * `aspect`, not pixels. A missing asset falls back to a placeholder box
   * with the `alt` text, so a broken image never breaks accessibility. */
  imageView() {
    const wrap = el("div", "pl-image");
    let key = null;
    return {
      el: wrap,
      update: (n) => {
        const src = n.props.get(Prop.src) ?? "";
        const alt = n.props.get(Prop.alt) ?? "";
        const aspectValue = n.props.get(Prop.aspect)?.enum ?? EnumAspect.square;
        const k = JSON.stringify([src, alt, aspectValue]);
        if (k === key) return;
        key = k;
        const ratio = ASPECT_RATIO[aspectValue] ?? ASPECT_RATIO[EnumAspect.square];
        const aspectClass = aspectValue === EnumAspect.wide ? "pl-image-wide" : aspectValue === EnumAspect.tall ? "pl-image-tall" : "pl-image-square";
        const url = this.assetUrl(src);
        wrap.className = `pl-image ${aspectClass}${url ? "" : " pl-image-placeholder"}`;
        wrap.style.aspectRatio = ratio;
        setAttr(wrap, "role", url ? null : "img");
        setAttr(wrap, "aria-label", url ? null : alt);
        if (url) {
          wrap.replaceChildren(el("img", "pl-image-img", { alt, src: url }));
        } else {
          const span = el("span", "pl-image-placeholder-text");
          span.textContent = alt;
          wrap.replaceChildren(span);
        }
      },
    };
  }

  /** `<Chart>` (SPEC.md §6.3, UI API 1.5): inline SVG with `<title>`/`<desc>`
   * for the accessible name and summary, plus a visually hidden `<table>`
   * so a screen reader gets every value. The runtime (this renderer), not
   * the app, picks the colors (the CSS chart palette, derived from the
   * accent and neutral tokens), height and axis. The drawing is made again
   * only when the chart's own props change. */
  chartView() {
    const wrap = el("div", "pl-chart");
    let key = null;
    return {
      el: wrap,
      update: (n) => {
        const k = JSON.stringify([Prop.label, Prop.chartKind, Prop.data, Prop.series].map((p) => n.props.get(p) ?? null));
        if (k === key) return;
        key = k;
        this.drawChart(wrap, n);
      },
    };
  }

  drawChart(wrap, n) {
    const label = n.props.get(Prop.label) ?? "";
    const kind = n.props.get(Prop.chartKind)?.enum ?? EnumChartKind.bar;
    const series = parseChartSeries(n);
    wrap.replaceChildren();

    if (series.length === 0 || series.every((s) => s.points.length === 0)) {
      setAttr(wrap, "role", "figure");
      setAttr(wrap, "aria-label", label);
      const empty = wrap.appendChild(el("div", "pl-chart-empty"));
      empty.textContent = "No data";
      return;
    }
    setAttr(wrap, "role", null);
    setAttr(wrap, "aria-label", null);

    wrap.appendChild(this.chartSvg(series, kind, label));
    if (kind === EnumChartKind.pie) {
      // Like the desktop: each label that has a wedge, and its share.
      const entries = pieLegend(series[0].points);
      if (entries.length > 0) wrap.appendChild(this.chartLegend(entries.map((e) => ({ color: e.index, name: e.label, extra: e.percent }))));
    } else if (series.length > 1) {
      wrap.appendChild(this.chartLegend(series.map((s, i) => ({ color: i, name: s.name }))));
    }
    wrap.appendChild(this.chartTable(series));
  }

  chartSvg(series, kind, label) {
    const W = 300, H = 150;
    const svg = svgEl("svg", { class: "pl-chart-svg", viewBox: `0 0 ${W} ${H}`, role: "img" });
    const uid = ++chartIds;
    svg.setAttribute("aria-labelledby", `pl-chart-title-${uid} pl-chart-desc-${uid}`);
    svg.appendChild(svgEl("title", { id: `pl-chart-title-${uid}` })).textContent = label;
    svg.appendChild(svgEl("desc", { id: `pl-chart-desc-${uid}` })).textContent = chartDescription(series);

    if (kind === EnumChartKind.pie) {
      const points = series[0].points;
      const mag = (v) => (Number.isFinite(v) ? Math.abs(v) : 0);
      const total = points.reduce((s, p) => s + mag(p.value), 0);
      const cx = W / 2, cy = H / 2, r = Math.min(W, H) / 2 - 6;
      if (total <= 0) {
        svg.appendChild(svgEl("circle", { cx, cy, r, class: "pl-chart-track" }));
        return svg;
      }
      let angle = -Math.PI / 2;
      points.forEach((p, i) => {
        const frac = mag(p.value) / total;
        if (frac <= 0) return;
        svg.appendChild(svgEl("path", { d: piePath(cx, cy, r, angle, frac), class: `pl-chart-c${i % 6}` }));
        angle += frac * Math.PI * 2;
      });
      return svg;
    }

    // Bar and line: one scale that always holds zero, so a negative value
    // goes below the zero line (the same scale as the desktop).
    const scale = niceScale(series.flatMap((s) => s.points.map((p) => p.value)));
    const labels = series[0].points.map((p) => p.label);
    const padL = 4, padR = 4, padB = 16, padT = 6;
    const plotW = W - padL - padR, plotH = H - padT - padB;
    const n = Math.max(1, labels.length);
    const yOf = (f) => padT + (1 - f) * plotH;

    if (kind === EnumChartKind.line) {
      // Each point sits in the center of its column, over its x label (as
      // on the desktop, `x_frac`).
      const xOf = (i) => padL + ((i + 0.5) / n) * plotW;
      series.forEach((s, si) => {
        const pts = s.points.map((p, i) => `${xOf(i)},${yOf(scaleFrac(scale, p.value))}`);
        svg.appendChild(svgEl("polyline", { points: pts.join(" "), class: `pl-chart-line pl-chart-c${si % 6}` }));
        s.points.forEach((p, i) => {
          svg.appendChild(svgEl("circle", { cx: xOf(i), cy: yOf(scaleFrac(scale, p.value)), r: 2.5, class: `pl-chart-c${si % 6}` }));
        });
      });
    } else {
      const groupW = plotW / n;
      const barW = groupW / (series.length + 1);
      labels.forEach((lbl, i) => {
        series.forEach((s, si) => {
          const span = barSpan(scale, s.points[i]?.value ?? 0);
          if (!span) return; // zero: no bar
          const x = padL + i * groupW + barW * (si + 0.5);
          svg.appendChild(svgEl("rect", { x, y: yOf(span[1]), width: barW * 0.85, height: (span[1] - span[0]) * plotH, class: `pl-chart-c${si % 6}` }));
        });
      });
    }
    // The zero line, drawn over the bars' ends.
    const zy = yOf(scaleFrac(scale, 0));
    svg.appendChild(svgEl("line", { x1: padL, x2: W - padR, y1: zy, y2: zy, class: "pl-chart-zero" }));
    labels.forEach((lbl, i) => {
      const x = padL + ((i + 0.5) / n) * plotW;
      svg.appendChild(svgEl("text", { x, y: H - 3, class: "pl-chart-axis-label", "text-anchor": "middle" })).textContent = lbl;
    });
    return svg;
  }

  /** `entries`: `[{ color, name, extra? }]`. */
  chartLegend(entries) {
    const legend = el("ul", "pl-chart-legend");
    for (const { color, name, extra } of entries) {
      const item = legend.appendChild(el("li", "pl-chart-legend-item"));
      item.appendChild(el("span", `pl-chart-legend-swatch pl-chart-c${color % 6}`));
      item.appendChild(el("span", "pl-chart-legend-label")).textContent = name;
      if (extra) {
        item.appendChild(document.createTextNode(" "));
        item.appendChild(el("span", "pl-chart-legend-value")).textContent = extra;
      }
    }
    return legend;
  }

  /** A visually hidden data table: a screen reader gets the exact numbers
   * without relying on the drawing. */
  chartTable(series) {
    const table = el("table", "pl-chart-table pl-sr-only");
    const multi = series.length > 1 && series.some((s) => s.name);
    const labels = series[0]?.points.map((p) => p.label) ?? [];
    const head = el("tr");
    head.appendChild(el("th"));
    if (multi) {
      for (const s of series) head.appendChild(el("th")).textContent = s.name;
    } else {
      head.appendChild(el("th"));
    }
    table.appendChild(el("thead")).appendChild(head);
    const body = table.appendChild(el("tbody"));
    labels.forEach((lbl, i) => {
      const row = body.appendChild(el("tr"));
      row.appendChild(el("th")).textContent = lbl;
      for (const s of multi ? series : [series[0]]) row.appendChild(el("td")).textContent = String(s.points[i]?.value ?? "");
    });
    return table;
  }

  /** `Tabs.items` arrive joined with U+001F; `value` is the selected item's
   * text. Arrow keys move the focus and select (roving tabindex), as the
   * desktop Tabs control does. */
  tabsView() {
    const wrap = el("div", "pl-tabs-wrap");
    const list = wrap.appendChild(el("div", "pl-tabs", { role: "tablist" }));
    let id = 0;
    let itemsKey = null;
    let tabs = []; // [item, button]
    list.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
      const i = tabs.findIndex(([, b]) => b === e.target);
      if (i < 0) return;
      e.preventDefault();
      const next = e.key === "ArrowRight" ? (i + 1) % tabs.length : (i - 1 + tabs.length) % tabs.length;
      tabs[next][1].focus();
      this.sendChange(id, tabs[next][0]);
      this.updateView(this.views.get(id));
    });
    return {
      el: wrap,
      slot: wrap,
      update: (n) => {
        id = n.id;
        const key = n.props.get(Prop.items) ?? "";
        if (key !== itemsKey) {
          itemsKey = key;
          tabs = key.split("\u001f").filter((s) => s.length > 0).map((item) => {
            const tab = el("button", "pl-tab", { role: "tab" });
            tab.textContent = item;
            tab.addEventListener("click", () => {
              this.sendChange(id, item);
              this.updateView(this.views.get(id));
            });
            return [item, tab];
          });
          list.replaceChildren(...tabs.map(([, b]) => b));
        }
        const current = n.props.get(Prop.value) ?? tabs[0]?.[0];
        for (const [item, tab] of tabs) {
          const selected = item === current;
          setAttr(tab, "aria-selected", selected ? "true" : "false");
          setProp(tab, "tabIndex", selected ? 0 : -1);
          tab.classList.toggle("pl-tab-selected", selected);
        }
      },
    };
  }

  /** `Sheet`/`Dialog` bind `open` (`value`) both ways (SPEC.md §6.3 UI API
   * 1.2): a native `<dialog>`. Escape or a close by the browser closes it
   * as the desktop does: `value` false in the tree, then `change` and
   * `close` to the guest. */
  overlayView(kname) {
    const dialog = el("dialog", `pl-${kname}`);
    const title = dialog.appendChild(el("h2", "pl-title"));
    let id = 0;
    const closedByUser = () => {
      const n = this.node(id);
      if (!n || !n.props.get(Prop.value)) return; // the guest closed it
      this.sendChange(id, false);
      this.fire(id, Event.close);
    };
    dialog.addEventListener("close", closedByUser);
    dialog.addEventListener("cancel", closedByUser); // Escape
    const sync = (dialog._plSync = () => {
      const open = !!this.node(id)?.props.get(Prop.value);
      if (!dialog.isConnected) return;
      if (open && !dialog.open) dialog.showModal();
      if (!open && dialog.open) dialog.close();
    });
    return {
      el: dialog,
      slot: dialog,
      update: (n) => {
        id = n.id;
        setOptionalText(title, n.props.get(Prop.title));
        // After the element is in the document (showModal needs that).
        queueMicrotask(sync);
      },
    };
  }

  /** `Menu` is an anchored popover (SPEC.md §6.3): a label button and its
   * `action` children as menu items. */
  menuView() {
    const wrap = el("div", "pl-menu");
    const trigger = wrap.appendChild(el("button", "pl-menu-trigger", { "aria-haspopup": "menu", "aria-expanded": "false" }));
    const popover = wrap.appendChild(el("div", "pl-menu-popover", { role: "menu" }));
    popover.hidden = true;
    const items = () => [...popover.children];
    const onDocClick = (e) => {
      if (!wrap.contains(e.target)) closeMenu(false);
    };
    const closeMenu = (focusTrigger = true) => {
      popover.hidden = true;
      trigger.setAttribute("aria-expanded", "false");
      document.removeEventListener("click", onDocClick);
      if (focusTrigger) trigger.focus();
    };
    // An item's own click (the Action) runs first, then this closes the menu.
    popover.addEventListener("click", (e) => {
      if (e.target.closest(".pl-menu-item")) closeMenu();
    });
    popover.addEventListener("keydown", (e) => {
      const all = items();
      const i = all.indexOf(document.activeElement);
      if (e.key === "Escape") {
        e.preventDefault();
        closeMenu();
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        all[(i + 1 + all.length) % all.length]?.focus();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        all[(i - 1 + all.length) % all.length]?.focus();
      }
    });
    trigger.addEventListener("click", () => {
      const willOpen = popover.hidden;
      popover.hidden = !willOpen;
      trigger.setAttribute("aria-expanded", willOpen ? "true" : "false");
      if (willOpen) {
        document.addEventListener("click", onDocClick);
        items()[0]?.focus();
      } else {
        document.removeEventListener("click", onDocClick);
      }
    });
    return {
      el: wrap,
      update: (n) => setText(trigger, n.props.get(Prop.label) ?? ""),
      place: (children) => {
        const actions = children.filter((c) => c.kind === ControlKind.action);
        for (const a of actions) {
          a.el.setAttribute("role", "menuitem");
          a.el.classList.add("pl-menu-item");
          a.el.tabIndex = -1;
        }
        reconcile(popover, actions.map((c) => c.el));
      },
    };
  }
}
