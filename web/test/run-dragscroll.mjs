// Node test of the auto-scroll speed during a drag (finding H3): the same
// numbers as `crates/plinth-ui/src/render.rs` (`drag_edge_speed`). The
// scroll itself runs in Edge (`run-a11y.mjs`, `checkDragDrop`).

import assert from "node:assert/strict";
import { edgeVelocity, DRAG_EDGE, DRAG_SPEED } from "../dom-renderer.js";

assert.equal(DRAG_EDGE, 40);
assert.equal(DRAG_SPEED, 600);
// A range 0..400: nothing in the middle or at the zone border.
assert.equal(edgeVelocity(200, 0, 400), 0);
assert.equal(edgeVelocity(40, 0, 400), 0);
assert.equal(edgeVelocity(360, 0, 400), 0);
// In proportion to the depth, signed.
assert.equal(edgeVelocity(380, 0, 400), DRAG_SPEED / 2);
assert.equal(edgeVelocity(20, 0, 400), -DRAG_SPEED / 2);
assert.equal(edgeVelocity(400, 0, 400), DRAG_SPEED);
assert.equal(edgeVelocity(0, 0, 400), -DRAG_SPEED);
// Outside the range: nothing.
assert.equal(edgeVelocity(-1, 0, 400), 0);
assert.equal(edgeVelocity(401, 0, 400), 0);
// A small range (0..60) has a zone of 20 px, a third of its size.
assert.equal(edgeVelocity(50, 0, 60), DRAG_SPEED / 2);
assert.equal(edgeVelocity(30, 0, 60), 0);
assert.equal(edgeVelocity(5, 10, 10), 0);
console.log("ok: drag auto-scroll speed");
