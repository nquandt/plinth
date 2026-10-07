// Node test of the Level 2 primitives on the web host (UI API 1.6,
// docs/UI-ADVANCED.md): the CSS that `primitiveStyle` gives for each prop
// (the same mapping as crates/plinth-ui/src/primitives.rs), and the
// example app `examples/primitives` through the protocol: its tree and a
// press on a Pressable.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/primitives
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { Tree, primitiveStyle, colorVar, UNIT, parsePartialStyle, partialRule, parseShapes } from "../dom-renderer.js";
import { ControlKind, Prop, Event, EnumAxis, EnumColor, EnumFraction, EnumRadius, EnumCrossAlign, EnumJustify, EnumTextSize, EnumWeight, EnumPosition } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

// Enum props arrive as `{ enum: value }`; `E(v)` makes one.
const E = (v) => ({ enum: v });
const node = (kind, props) => ({ kind, props: new Map(Object.entries(props).map(([k, v]) => [Prop[k], v])) });

// -- The mapping ---------------------------------------------------------------
assert.equal(UNIT, 4);
assert.equal(colorVar(EnumColor.textMuted), "var(--pl-muted)");
assert.equal(colorVar(EnumColor.none), null);

const box = primitiveStyle(
  node(ControlKind.box, {
    axis: E(EnumAxis.row),
    wrap: true,
    gap: 3,
    padding: 2,
    paddingX: 4,
    crossAlign: E(EnumCrossAlign.center),
    justify: E(EnumJustify.between),
    grow: 2,
    width: 40,
    maxHeightFraction: E(EnumFraction.half),
    bg: E(EnumColor.surfaceAlt),
    border: E(EnumColor.danger),
    radius: E(EnumRadius.full),
  }),
);
assert.deepEqual(box, {
  flex: "2 1 0px",
  "min-width": "0",
  display: "flex",
  "flex-direction": "row",
  "flex-wrap": "wrap",
  gap: "12px",
  padding: "8px",
  "padding-inline": "16px",
  "align-items": "center",
  "justify-content": "space-between",
  "flex-shrink": "0",
  width: "160px",
  "max-height": "50%",
  background: "var(--pl-surface-alt)",
  border: "1px solid var(--pl-danger)",
  "border-radius": "9999px",
});

// Defaults: a column that stretches its children; units are clamped.
const plain = primitiveStyle(node(ControlKind.box, { padding: -3, height: 100000 }));
assert.equal(plain["flex-direction"], "column");
assert.equal(plain["align-items"], "stretch");
assert.equal(plain.padding, "0px");
assert.equal(plain.height, `${512 * 4}px`);

assert.equal(primitiveStyle(node(ControlKind.scroll, {}))["overflow-y"], "auto");
assert.equal(primitiveStyle(node(ControlKind.scroll, { axis: E(EnumAxis.row) }))["overflow-x"], "auto");

const span = primitiveStyle(node(ControlKind.span, { textSize: E(EnumTextSize.xxl), weight: E(EnumWeight.bold), italic: true, fg: E(EnumColor.accent), lines: 2 }));
assert.equal(span["font-size"], "1.5rem");
assert.equal(span["font-weight"], "700");
assert.equal(span["font-style"], "italic");
assert.equal(span.color, "var(--pl-accent-fg)");
assert.equal(span["-webkit-line-clamp"], "2");
assert.equal(primitiveStyle(node(ControlKind.span, {})).color, "var(--pl-text)");

// -- Partial styles (UI API 1.7) ----------------------------------------------
const parsed = parsePartialStyle(`${Prop.bg}:${EnumColor.hover}, ${Prop.padding}:4,bad,${Prop.wrap}:1,${Prop.gap}:-1`);
assert.deepEqual([...parsed], [
  [Prop.bg, { enum: EnumColor.hover }],
  [Prop.padding, 4],
  [Prop.wrap, true],
  [Prop.gap, -1],
]);
// UI API 1.11: sizes and insets can be fractional units.
assert.deepEqual([...parsePartialStyle(`${Prop.left}:2.25,${Prop.width}:-0.5`)], [
  [Prop.left, 2.25],
  [Prop.width, -0.5],
]);
const moving = primitiveStyle(node(ControlKind.box, { position: E(EnumPosition.absolute), left: 10.25, top: 0.9, width: 2.5 }));
assert.equal(moving.left, "41px");
assert.equal(moving.top, `${0.9 * UNIT}px`);
assert.equal(moving.width, "10px");
// A partial style sets only what it names: no display, no default direction.
assert.deepEqual(primitiveStyle({ kind: ControlKind.box, props: parsePartialStyle(`${Prop.border}:${EnumColor.accent}`) }, true), {
  border: "1px solid var(--pl-accent)",
});
assert.equal(
  partialRule(ControlKind.pressable, Prop.hover, `${Prop.bg}:${EnumColor.hover}`, "pl-st-1"),
  ".pl-st-1:hover { background: var(--pl-hover) !important; }",
);
assert.equal(
  partialRule(ControlKind.box, Prop.compact, `${Prop.axis}:${EnumAxis.column}`, "pl-st-2"),
  "@media (max-width: 599.98px) { .pl-st-2 { flex-direction: column !important; } }",
);
assert.equal(partialRule(ControlKind.box, Prop.wide, "garbage", "pl-st-3"), null);

// -- The example app -----------------------------------------------------------
const pkg = new Uint8Array(readFileSync(path.join(root, "examples/primitives/dist/primitives.plnt")));
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const { appWasm, manifestText } = await readPlnt(pkg);
const app = new PlinthApp();
const tree = new Tree();
app.onCommit = (ops) => tree.apply(ops);
await app.load(core, appWasm, { log: () => {}, manifestText });
app.init([]);

const all = [...tree.nodes.values()];
const kinds = new Set(all.map((n) => n.kind));
for (const k of ["box", "span", "pressable", "scroll"]) assert.ok(kinds.has(ControlKind[k]), `the app has a ${k}`);
const texts = () => all.filter((n) => n.kind === ControlKind.span).map((n) => tree.nodes.get(n.id)?.text ?? "");
const pressable = (label) => [...tree.nodes.values()].find((n) => n.kind === ControlKind.pressable && n.props.get(Prop.label) === label);

const pressMe = pressable("Press me");
assert.ok(pressMe, "a Pressable labelled Press me");
assert.equal(pressMe.props.get(Prop.bg)?.enum, EnumColor.accent);
app.onEvent({ kind: "ui", handler: pressMe.listeners.get(Event.press), event: Event.press, value: null });
app.onEvent({ kind: "ui", handler: pressMe.listeners.get(Event.press), event: Event.press, value: null });
assert.ok(texts().includes("Pressed 2 times"), `after two presses: ${texts().join(" | ")}`);
assert.equal(pressable("Reset the count").props.get(Prop.disabled), false);

// A card in the Scroll: pressing it selects it (a token prop changes).
const inbox = pressable("Inbox");
assert.equal(inbox.props.get(Prop.bg)?.enum, EnumColor.surface);
app.onEvent({ kind: "ui", handler: inbox.listeners.get(Event.press), event: Event.press, value: null });
assert.equal(pressable("Inbox").props.get(Prop.bg)?.enum, EnumColor.selected);
assert.ok([...tree.nodes.values()].some((n) => n.text === "Chosen: Inbox"));

// Canvas (UI API 1.10): the compiler encodes the shapes; the circle grows with the presses.
const canvasNode = () => [...tree.nodes.values()].find((n) => n.kind === ControlKind.canvas);
const shapes = parseShapes(canvasNode().props.get(Prop.shapes));
assert.equal(shapes.length, 9, JSON.stringify(shapes));
assert.deepEqual(shapes[0], { kind: "rect", x: 0, y: 0, w: 200, h: 100, color: "surface.alt" });
assert.deepEqual(shapes[2], { kind: "line", x1: 0, y1: 80, x2: 200, y2: 80, color: "border", width: 2 });
assert.deepEqual(shapes[3], { kind: "rect", x: 14, y: 62, w: 14, h: 18, color: "success" });
assert.deepEqual(shapes[8], { kind: "text", x: 8, y: 96, color: "text.muted", size: 9, text: "Canvas: view 200 x 100" });
assert.equal(shapes[1].r, 12 + 2, "the sun has grown by the two presses");
assert.equal(canvasNode().props.get(Prop.viewWidth), 200);

// The compiler encodes the partial styles of the app.
assert.equal(pressable("Inbox").props.get(Prop.hover), `${Prop.border}:${EnumColor.accent}`);
const header = [...tree.nodes.values()].find((n) => n.kind === ControlKind.box && n.props.get(Prop.compact));
assert.equal(header.props.get(Prop.compact), `${Prop.axis}:${EnumAxis.column},${Prop.crossAlign}:${EnumCrossAlign.start}`);

// The style of real nodes (the wire form of each prop type: enum, int, bool).
const scroll = [...tree.nodes.values()].find((n) => n.kind === ControlKind.scroll);
assert.deepEqual(
  [primitiveStyle(scroll)["flex-direction"], primitiveStyle(scroll).gap, primitiveStyle(scroll)["overflow-x"]],
  ["row", "12px", "auto"],
);
const card = primitiveStyle(pressable("Photos"));
assert.equal(card.width, "160px");
assert.equal(card["border-radius"], "8px");
const wrapRow = [...tree.nodes.values()].find((n) => n.kind === ControlKind.box && n.props.get(Prop.wrap) === true);
assert.equal(primitiveStyle(wrapRow)["flex-wrap"], "wrap");
const third = [...tree.nodes.values()].find((n) => n.kind === ControlKind.box && n.props.get(Prop.widthFraction));
assert.equal(primitiveStyle(third).width, "33.3333%");

console.log("run-primitives.mjs: ok (style mapping, Box/Span/Pressable/Scroll in examples/primitives, presses)");
