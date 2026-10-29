// Node test of UI API 1.14 on the web host: the CSS that `primitiveStyle`
// and `partialRule` give for `hidden`, `minWidth` and `minHeight` (the same
// mapping as crates/plinth-ui/src/primitives.rs). The DOM facts (aria-pressed,
// aria-selected, aria-current, layout full, the live width class) are in
// `run-a11y.mjs` `checkUi114`.

import assert from "node:assert/strict";
import { primitiveStyle, parsePartialStyle, partialRule, UNIT } from "../dom-renderer.js";
import { ControlKind, Prop, EnumFraction, UI_API_VERSION } from "../ui-api.js";

const E = (v) => ({ enum: v });
const node = (kind, props) => ({ kind, props: new Map(Object.entries(props).map(([k, v]) => [Prop[k], v])) });

assert.equal(UI_API_VERSION, "1.14");
assert.equal(UNIT, 4);

// hidden: display none, for every primitive; false adds nothing.
for (const kind of [ControlKind.box, ControlKind.scroll, ControlKind.pressable, ControlKind.span]) {
  assert.equal(primitiveStyle(node(kind, { hidden: true })).display, "none", `kind ${kind}`);
}
assert.equal(primitiveStyle(node(ControlKind.box, { hidden: false })).display, "flex");
assert.equal(primitiveStyle(node(ControlKind.span, { hidden: false })).display, undefined);

// min sizes: units and fractions; after `grow`, which sets a zero floor.
const floors = primitiveStyle(node(ControlKind.box, { grow: 1, minWidth: 52, minHeightFraction: E(EnumFraction.half) }));
assert.equal(floors["min-width"], `${52 * UNIT}px`);
assert.equal(floors["min-height"], "50%");
assert.equal(primitiveStyle(node(ControlKind.box, { grow: 1 }))["min-width"], "0");
assert.equal(primitiveStyle(node(ControlKind.box, { minWidth: 2.5 }))["min-width"], "10px");

// Partial styles: the compiler gives `<id>:1` for `hidden: true`.
assert.deepEqual([...parsePartialStyle(`${Prop.hidden}:1,${Prop.minWidthFraction}:${EnumFraction.third}`)], [
  [Prop.hidden, true],
  [Prop.minWidthFraction, { enum: EnumFraction.third }],
]);
assert.equal(
  partialRule(ControlKind.box, Prop.compact, `${Prop.hidden}:1`, "pl-st-1"),
  "@media (max-width: 599.98px) { .pl-st-1 { display: none !important; } }",
);
// `hidden: false` in a width class shows the element again over a base `hidden`.
assert.equal(
  partialRule(ControlKind.pressable, Prop.wide, `${Prop.hidden}:0`, "pl-st-2"),
  "@media (min-width: 1200px) { .pl-st-2 { display: flex !important; } }",
);
assert.equal(
  partialRule(ControlKind.span, Prop.regular, `${Prop.hidden}:0`, "pl-st-3"),
  "@media (min-width: 600px) and (max-width: 1199.98px) { .pl-st-3 { display: var(--pl-show, revert) !important; } }",
);
// A line-clamped Span stores its display, so a shown Span is clamped again.
assert.equal(primitiveStyle(node(ControlKind.span, { lines: 1, hidden: true }))["--pl-show"], "-webkit-box");
assert.equal(primitiveStyle(node(ControlKind.span, { lines: 1, hidden: true })).display, "none");
assert.equal(primitiveStyle(node(ControlKind.span, { lines: 2 })).display, "-webkit-box");
assert.equal(primitiveStyle(node(ControlKind.span, {}))["--pl-show"], "revert");
assert.equal(primitiveStyle(node(ControlKind.span, { hidden: false }), true).display, "var(--pl-show, revert)");
assert.equal(
  partialRule(ControlKind.box, Prop.compact, `${Prop.minWidth}:30`, "pl-st-4"),
  "@media (max-width: 599.98px) { .pl-st-4 { min-width: 120px !important; } }",
);

console.log("ok: UI API 1.14 style mapping (hidden, minWidth, minHeight)");
