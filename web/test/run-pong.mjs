// Node test of examples/pong on the web host: it starts the game, steps the
// game loop by hand (the app's setInterval goes to a fake timer that the
// test fires), presses the paddle buttons, and reads the court from the
// tree: the ball moves, bounces from the walls and the paddles, and a
// point is scored. It also measures the time of one tick (event + commit).
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/pong
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync, statSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Prop, Event, EnumColor } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

// The court (examples/pong/app/game.ts), in spacing units.
const FIELD_H = 48, PADDLE_H = 10, BALL = 2, HALF_W = 37, NET_W = 1, FIELD_W = 75;

// A fake setInterval: the host's `set-timer` calls the global functions, so
// the test owns the clock. `tickAll()` fires every live interval once.
const intervals = new Map();
let nextHandle = 1;
globalThis.setInterval = (fn, _ms) => {
  const h = nextHandle++;
  intervals.set(h, fn);
  return h;
};
globalThis.clearInterval = (h) => intervals.delete(h);
const tickAll = () => {
  for (const fn of [...intervals.values()]) fn();
};

const plntPath = path.join(root, "examples/pong/dist/pong.plnt");
const pkg = new Uint8Array(readFileSync(plntPath));
const core = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
const { appWasm, manifestText } = await readPlnt(pkg);
const app = new PlinthApp();
const tree = new Tree();
let opCount = 0;
app.onCommit = (ops) => {
  opCount += ops.length;
  tree.apply(ops);
};
await app.load(core, appWasm, { log: () => {}, manifestText });
app.init([]);

const nodes = () => [...tree.nodes.values()];
const labelled = (label) => {
  const n = nodes().find((n) => n.props.get(Prop.label) === label);
  assert.ok(n, `a node labelled ${label}`);
  return n;
};
const press = (label) => {
  const p = nodes().find((n) => n.kind === ControlKind.pressable && n.props.get(Prop.label) === label);
  assert.ok(p, `a Pressable labelled ${label}`);
  app.onEvent({ kind: "ui", handler: p.listeners.get(Event.press), event: Event.press, value: null });
};
const height = (n) => n.props.get(Prop.height) ?? 0;
const width = (n) => n.props.get(Prop.width) ?? 0;
/** The spacer before `n` in its parent (the Box that places it). */
const spacerBefore = (n) => {
  const parent = tree.nodes.get(n.parent);
  return tree.nodes.get(parent.children[parent.children.indexOf(n.id) - 1]);
};
const paddleTop = (label) => height(spacerBefore(labelled(label)));
/** The ball's court position, read from the spacers of the half that draws it. */
const ball = () => {
  for (const [half, dx] of [["Ball (left half)", 0], ["Ball (right half)", HALF_W + NET_W]]) {
    const b = labelled(half);
    if (b.props.get(Prop.bg)?.enum === EnumColor.none) continue;
    const row = tree.nodes.get(b.parent);
    return { x: dx + width(spacerBefore(b)), y: height(spacerBefore(row)) };
  }
  assert.fail("no ball is visible");
};
const texts = () => nodes().filter((n) => n.kind === ControlKind.span).map((n) => n.text ?? "");
const scores = () => {
  const t = texts();
  const you = t.indexOf("You"), cpu = t.indexOf("Computer");
  return [Number(t[you + 1]), Number(t[cpu + 1])];
};

// -- Before the start -------------------------------------------------------
assert.ok(texts().includes("First to 5 wins. Press Start."), texts().join(" | "));
assert.deepEqual(scores(), [0, 0]);
const centerY = (FIELD_H - PADDLE_H) / 2;
assert.equal(paddleTop("Your paddle"), centerY);
const start = ball();
// The ball starts on the net; the right half draws it at its left edge.
assert.deepEqual(start, { x: HALF_W + NET_W, y: Math.round((FIELD_H - BALL) / 2) });
assert.equal(intervals.size, 0, "no game loop before Start");

// -- Start: the ball moves toward the computer ---------------------------------
press("Start");
assert.equal(intervals.size, 1, "Start starts the game loop");
assert.ok(labelled("Pause"), "the start button now pauses");
for (let i = 0; i < 10; i++) tickAll();
const moved = ball();
assert.ok(moved.x > start.x, `the ball moves right: ${JSON.stringify([start, moved])}`);
assert.notEqual(moved.y, start.y, "the ball also moves up or down");

// Pause stops the loop; Resume starts it again.
press("Pause");
assert.equal(intervals.size, 0);
tickAll();
assert.deepEqual(ball(), moved);
press("Resume");
assert.equal(intervals.size, 1);

// -- The player's paddle -------------------------------------------------------
press("Up");
for (let i = 0; i < 5; i++) tickAll();
const up = paddleTop("Your paddle");
assert.ok(up < centerY, `Up moves the paddle up: ${up}`);
press("Stop");
tickAll();
assert.equal(paddleTop("Your paddle"), up, "Stop keeps it still");
press("Down");
for (let i = 0; i < 40; i++) tickAll();
assert.equal(paddleTop("Your paddle"), FIELD_H - PADDLE_H, "Down moves it to the bottom wall and no further");
press("Stop");

// -- Bounces and a point --------------------------------------------------------
// Follow the ball until the first point: watch for a wall bounce (vertical
// direction flips at a wall) and a paddle bounce (horizontal direction flips
// at the computer's side).
let prev = ball();
let dy = 0, dx = 0;
let wallBounce = false, computerReturn = false;
let tickTimes = [];
let ticks = 0;
const ops0 = opCount;
const points = () => scores()[0] + scores()[1];
while ((!wallBounce || !computerReturn || points() === 0) && ticks < 3000 && intervals.size > 0) {
  // Keep the player's paddle away from the ball, so the computer scores.
  const t0 = performance.now();
  tickAll();
  tickTimes.push(performance.now() - t0);
  ticks++;
  const now = ball();
  const ndx = Math.sign(now.x - prev.x), ndy = Math.sign(now.y - prev.y);
  if (dy !== 0 && ndy !== 0 && ndy !== dy && (now.y <= 1 || now.y >= FIELD_H - BALL - 1)) wallBounce = true;
  if (dx > 0 && ndx < 0 && prev.x >= FIELD_W - BALL - 2 && now.x >= FIELD_W - BALL - 6) computerReturn = true;
  if (ndx !== 0) dx = ndx;
  if (ndy !== 0) dy = ndy;
  prev = now;
}
const opsPerTick = (opCount - ops0) / ticks;
assert.ok(wallBounce, "the ball bounced from the top or bottom wall");
assert.ok(computerReturn, "the computer's paddle returned the ball");
assert.ok(scores()[1] >= 1, `the computer scores when the player misses (after ${ticks} ticks: ${scores()})`);

// -- The player returns a ball: put the paddle in the ball's path ----------------
let playerReturn = false;
for (let i = 0; i < 3000 && !playerReturn && scores()[1] < 5; i++) {
  const b = ball(), p = paddleTop("Your paddle");
  const want = b.y + BALL / 2 - PADDLE_H / 2;
  const target = want < p - 1 ? "Up" : want > p + 1 ? "Down" : "Stop";
  press(target);
  const before = ball();
  tickAll();
  const after = ball();
  if (before.x <= 3 && after.x > before.x) playerReturn = true;
}
press("Stop");
assert.ok(playerReturn, "the player's paddle returned the ball");

// -- To the end: the computer wins when the player stands still ------------------
for (let i = 0; i < 20000 && intervals.size > 0; i++) tickAll();
assert.equal(intervals.size, 0, "the loop stops when the game is over");
const [you, cpu] = scores();
assert.ok(you === 5 || cpu === 5, `one side has 5 points: ${you}:${cpu}`);
assert.ok(texts().includes(cpu === 5 ? "The computer wins." : "You win!"));
press("Play again");
assert.deepEqual(scores(), [0, 0]);
assert.equal(intervals.size, 1);

// -- Measurements ------------------------------------------------------------------
tickTimes.sort((a, b) => a - b);
const median = tickTimes[tickTimes.length >> 1];
const p95 = tickTimes[Math.floor(tickTimes.length * 0.95)];
const size = statSync(plntPath).size;
console.log(
  `run-pong.mjs: ok (move, pause, paddle, wall and paddle bounces, points, win; ` +
    `tick ${median.toFixed(3)} ms median, ${p95.toFixed(3)} ms p95 over ${tickTimes.length} ticks, ${opsPerTick.toFixed(1)} ops per tick; .plnt ${size} B)`,
);
process.exit(0);
