// Pie wedge paths of the web Chart renderer (dom-renderer.js `piePath`).
// A wedge over half the circle needs the SVG large-arc flag; without it the
// browser draws the short arc and the pie breaks (budget: Salary 70 %).
import assert from "node:assert/strict";
import { piePath } from "../dom-renderer.js";

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

console.log("run-chart-geometry.mjs: ok");
