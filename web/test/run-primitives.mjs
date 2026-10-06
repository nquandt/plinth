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
import { Tree, primitiveStyle, colorVar, UNIT } from "../dom-renderer.js";
import { ControlKind, Prop, Event, EnumAxis, EnumColor, EnumFraction, EnumRadius, EnumCrossAlign, EnumJustify, EnumTextSize, EnumWeight } from "../ui-api.js";

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
