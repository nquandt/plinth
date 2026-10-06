// Node test (docs/VALIDATION.md V1): the 7GUIs tasks 1-5 in
// examples/7guis/, on the web host's code: the same PlinthApp and the same
// semantic tree (dom-renderer.js `Tree`) that the browser uses, with no DOM.
// A `change` is sent as `DomRenderer.sendChange` sends it: the host keeps
// the typed value in its tree, then sends the event.
// crates/plinth-compiler/tests/sevenguis.rs checks the same apps on the
// desktop runner.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/7guis/<name>   (for each app)
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Event, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));

/** One running app and its tree, with helpers to find nodes and send events. */
class Harness {
  static async start(name) {
    const plnt = new Uint8Array(readFileSync(path.join(root, `examples/7guis/${name}/dist/${name}.plnt`)));
    const { appWasm } = await readPlnt(plnt);
    const h = new Harness();
    h.errors = [];
    h.app = new PlinthApp();
    h.tree = new Tree();
    h.ops = [];
    h.app.onCommit = (ops) => {
      h.ops.push(...ops);
      h.tree.apply(ops);
    };
    await h.app.load(coreBytes, appWasm, { log: () => {}, reportError: (m) => h.errors.push(m) });
    h.app.init([]);
    return h;
  }

  /** The nodes of `kind` under the current screen, in tree order. */
  find(kind, pred = () => true) {
    const out = [];
    const rootId = this.tree.roots.get(this.tree.topScreen());
    const stack = rootId ? [rootId] : [];
    while (stack.length) {
      const n = this.tree.node(stack.pop());
      if (!n) continue;
      if (n.kind === kind && pred(n)) out.push(n);
      stack.push(...[...n.children].reverse());
    }
    return out;
  }

  one(kind, pred) {
    const f = this.find(kind, pred);
    assert.equal(f.length, 1, `expected one node of kind ${kind}, found ${f.length}`);
    return f[0];
  }

  labeled(kind, label) {
    return this.one(kind, (n) => n.props.get(Prop.label) === label);
  }

  send(node, event, value) {
    const handler = node.listeners.get(event);
    assert.ok(handler !== undefined, `node ${node.id} has no listener for event ${event}`);
    this.ops = [];
    this.app.onEvent({ kind: "ui", handler, event, value });
    assert.deepEqual(this.errors, [], "the app reported errors");
    return this.ops;
  }

  /** Types `text` into a text field, as DomRenderer.sendChange does. */
  type(node, text) {
    node.props.set(Prop.value, text);
    return this.send(node, Event.change, text);
  }

  press(label) {
    const b = this.labeled(ControlKind.button, label);
    assert.ok(!b.props.get(Prop.disabled), `${label} is disabled`);
    this.send(b, Event.press, null);
  }

  disabled(label) {
    return Boolean(this.labeled(ControlKind.button, label).props.get(Prop.disabled));
  }

  field(label) {
    return this.labeled(ControlKind.textField, label);
  }

  texts() {
    return this.find(ControlKind.text).map((n) => String(n.text ?? ""));
  }

  rowTitles() {
    return this.find(ControlKind.row).map((n) => n.props.get(Prop.title));
  }

  /** Fires every live interval once (see `fakeIntervals`). */
  tick() {
    for (const t of intervals.filter((t) => t.live)) t.fire();
    assert.deepEqual(this.errors, [], "the app reported errors");
  }
}

// The web host's `set-timer` uses the JS `setInterval`. The test replaces it,
// so that a tick happens when the test says so, not on a real clock.
const intervals = [];
globalThis.setInterval = (fire, ms) => {
  const t = { fire, ms, live: true };
  intervals.push(t);
  return t;
};
globalThis.clearInterval = (t) => {
  if (t) t.live = false;
};

const value = (n) => n.props.get(Prop.value) ?? "";
const error = (n) => n.props.get(Prop.error) ?? "";
const setsValue = (ops, node) => ops.some((op) => op.op === "set-prop" && op.id === node.id && op.prop === Prop.value);

async function counter() {
  const h = await Harness.start("counter");
  const heading = h.one(ControlKind.heading);
  assert.equal(String(heading.text), "0");
  h.press("Count");
  h.press("Count");
  assert.equal(String(heading.text), "2");
}

async function temperature() {
  const h = await Harness.start("temperature");
  const c = h.field("Celsius");
  const f = h.field("Fahrenheit");
  let ops = h.type(c, "100");
  assert.equal(value(f), "212");
  assert.ok(setsValue(ops, f));
  assert.ok(!setsValue(ops, c), "the typed field got its own value back");
  h.type(f, "32");
  assert.equal(value(c), "0");
  h.type(f, "98.6");
  assert.equal(value(c), "37");

  // Invalid text stays, shows an error, and does not change the other field.
  ops = h.type(c, "37a");
  assert.equal(value(c), "37a");
  assert.equal(value(f), "98.6");
  assert.equal(error(c), "Not a number");
  assert.ok(!setsValue(ops, c) && !setsValue(ops, f));
  h.type(f, "50");
  assert.equal(value(c), "10");
  assert.equal(error(c), "");
}

async function flightBooker() {
  const h = await Harness.start("flight-booker");
  const start = h.field("Start date");
  assert.equal(value(start).length, 10);
  assert.ok(!h.disabled("Book"));
  assert.equal(h.find(ControlKind.textField, (n) => n.props.get(Prop.label) === "Return date").length, 0);

  h.type(start, "31.04.2027");
  assert.equal(error(start), "Use the form DD.MM.YYYY");
  assert.ok(h.disabled("Book"));
  h.type(start, "10.03.2027");
  assert.equal(error(start), "");
  assert.ok(!h.disabled("Book"));

  h.send(h.labeled(ControlKind.picker, "Flight"), Event.change, "return flight");
  const back = h.field("Return date");
  h.type(back, "09.03.2027");
  assert.equal(error(back), "The return date is before the start date");
  assert.ok(h.disabled("Book"));
  h.type(back, "11.03.2027");
  assert.ok(!h.disabled("Book"));

  const dialog = h.one(ControlKind.dialog);
  assert.ok(!dialog.props.get(Prop.value));
  h.press("Book");
  assert.equal(dialog.props.get(Prop.value), true);
  assert.equal(dialog.props.get(Prop.message), "You booked a return flight on 10.03.2027, back on 11.03.2027.");
}



async function main() {
  for (const [name, test] of Object.entries({ counter, temperature, flightBooker })) {
    await test();
    console.log(`run-7guis.mjs: ${name} ok`);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
