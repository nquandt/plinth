// Chart geometry of the web renderer (dom-renderer.js): pie wedge paths,
// the y scale, bars around the zero line and the pie legend.
// A wedge over half the circle needs the SVG large-arc flag; without it the
// browser draws the short arc and the pie breaks (budget: Salary 70 %).
import assert from "node:assert/strict";
import { piePath, niceScale, scaleFrac, barSpan, pieLegend } from "../dom-renderer.js";

/** The large-arc flag of a wedge path: "M cx cy L x1 y1 A r r 0 <flag> 1 x2 y2 Z". */
function largeArc(d) {
  const m = / A [\d.e-]+ [\d.e-]+ 0 ([01]) 1 /.exec(d);
  assert.ok(m, `not a wedge path: ${d}`);
  return m[1];
}

const top = -Math.PI / 2;
assert.equal(largeArc(piePath(150, 75, 69, top, 0.25)), "0");
assert.equal(largeArc(piePath(150, 75, 69, top, 0.5)), "0");
assert.equal(largeArc(piePath(150, 75, 69, top, 0.7)), "1");

// The wedge ends where the next one starts: 70 % from 12 o'clock ends at
// 252° clockwise, which is left of the center and below it (screen y grows down).
const d = piePath(150, 75, 69, top, 0.7);
const [x2, y2] = d.trim().split(/\s+/).slice(-3, -1).map(Number);
const a = top + 0.7 * Math.PI * 2;
assert.ok(Math.abs(x2 - (150 + 69 * Math.cos(a))) < 1e-9 && Math.abs(y2 - (75 + 69 * Math.sin(a))) < 1e-9);
assert.ok(x2 < 150 && y2 > 75);

// A full circle is one closed arc.
assert.match(piePath(150, 75, 69, top, 1), /^M 150 6 A 69 69 0 1 1 /);

// The y scale holds zero and every value, like `Scale::for_values` on the desktop.
assert.deepEqual(niceScale([120, 950, 400]), { lo: 0, hi: 1000, step: 250 });
assert.deepEqual(niceScale([]), { lo: 0, hi: 1, step: 0.25 });
const neg = niceScale([-30, 70]);
assert.ok(neg.lo <= -30 && neg.hi >= 70 && neg.lo < 0, JSON.stringify(neg));

// Bars start at the zero line: up for a positive value, down for a
// negative one, and no bar (not even 1 px) for zero.
const s = { lo: -50, hi: 100, step: 25 };
const zero = scaleFrac(s, 0);
assert.ok(Math.abs(zero - 1 / 3) < 1e-12);
assert.deepEqual(barSpan(s, 100), [zero, 1]);
assert.deepEqual(barSpan(s, -50), [0, zero]);
const [from, to] = barSpan(s, -25);
assert.ok(to === zero && from > 0 && from < zero, "a negative bar hangs below zero");
assert.equal(barSpan(s, 0), null);
assert.equal(barSpan(s, Number.NaN), null);

// The pie legend: only the points with a wedge, with whole percents.
assert.deepEqual(
  pieLegend([{ label: "A", value: 1 }, { label: "Zero", value: 0 }, { label: "B", value: 3 }]),
  [{ index: 0, label: "A", percent: "25%" }, { index: 2, label: "B", percent: "75%" }]
);
assert.deepEqual(pieLegend([{ label: "Tiny", value: 1 }, { label: "Big", value: 999 }]).map((e) => e.percent), ["<1%", "100%"]);
assert.deepEqual(pieLegend([{ label: "None", value: 0 }]), []);
assert.deepEqual(pieLegend([{ label: "Refund", value: -1 }, { label: "Rent", value: 3 }]).map((e) => e.percent), ["25%", "75%"]);

console.log("run-chart-geometry.mjs: ok");
