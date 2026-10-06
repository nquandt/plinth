// The web renderer's semantic tree (dom-renderer.js `Tree`), with no DOM:
// the op semantics of SPEC.md §8.2 and the change record that lets the
// renderer touch only the nodes a commit changed.
import assert from "node:assert/strict";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Prop } from "../ui-api.js";

const tree = new Tree();
let commits = 0;
tree.onChange = () => commits++;

const L = ControlKind.list, R = ControlKind.row, T = ControlKind.text;
tree.apply([
  { op: "create", id: 1, kind: ControlKind.screen },
  { op: "create", id: 2, kind: L },
  { op: "insert", parent: 1, id: 2, before: 0 },
  ...[10, 11, 12].flatMap((id) => [
    { op: "create", id, kind: R },
    { op: "create", id: id + 100, kind: T },
    { op: "insert", parent: id, id: id + 100, before: 0 },
    { op: "insert", parent: 2, id, before: 0 },
  ]),
  { op: "set-root", screen: 0, id: 1 },
]);
assert.equal(commits, 1);
assert.deepEqual(tree.node(2).children, [10, 11, 12]);
let ch = tree.takeChanges();
assert.ok(ch.nav && ch.children.has(2) && ch.created.has(12));

// One prop: only that node is dirty; no structure changed.
tree.apply([{ op: "set-prop", id: 11, prop: Prop.title, value: "B" }]);
ch = tree.takeChanges();
assert.deepEqual([...ch.dirty], [11]);
assert.equal(ch.children.size, 0);
assert.equal(ch.nav, false);

// A move inside the parent, and an insert before an anchor.
tree.apply([{ op: "move", parent: 2, id: 12, before: 10 }]);
assert.deepEqual(tree.node(2).children, [12, 10, 11]);
ch = tree.takeChanges();
assert.deepEqual([...ch.children], [2]);

// A move to another parent leaves the old one (both change).
tree.apply([
  { op: "create", id: 3, kind: L },
  { op: "insert", parent: 1, id: 3, before: 0 },
  { op: "move", parent: 3, id: 10, before: 0 },
]);
assert.deepEqual(tree.node(2).children, [12, 11]);
assert.deepEqual(tree.node(3).children, [10]);
assert.equal(tree.node(10).parent, 3);
ch = tree.takeChanges();
assert.ok(ch.children.has(2) && ch.children.has(3) && ch.children.has(1));

// remove frees the whole subtree (SPEC.md §8.2), and an id can be used again.
tree.apply([{ op: "remove", id: 11 }]);
assert.equal(tree.node(11), undefined);
assert.equal(tree.node(111), undefined, "the row's text goes with the row");
assert.deepEqual(tree.node(2).children, [12]);
ch = tree.takeChanges();
assert.deepEqual([...ch.removed].sort(), [11, 111]);
assert.ok(ch.children.has(2));
tree.apply([
  { op: "create", id: 11, kind: T },
  { op: "insert", parent: 2, id: 11, before: 12 },
]);
assert.deepEqual(tree.node(2).children, [11, 12]);
ch = tree.takeChanges();
assert.ok(ch.created.has(11) && !ch.removed.has(11));

// Removing a screen root clears the root.
tree.apply([{ op: "remove", id: 1 }]);
assert.equal(tree.roots.size, 0);
assert.equal(tree.nodes.size, 0);
assert.ok(tree.takeChanges().nav);

console.log("run-tree.mjs: ok");
