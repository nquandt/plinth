// Node test of examples/pong on the web host: it starts the game, steps the
// game loop by hand (the app's onFrame loop, core 1.12, goes to a fake
// requestFrame that the test fires as a 60 Hz display), presses the paddle buttons, and reads the court from the
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
import { PlinthApp, readPlnt, INIT_RANDOM_SEED } from "../plinth-web.js";
import { Tree } from "../dom-renderer.js";
import { ControlKind, Prop, Event, EnumColor } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

// The court (examples/pong/app/game.ts), in spacing units.
const FIELD_H = 48, PADDLE_H = 10, PADDLE_W = 2, BALL = 2, HALF_W = 37, NET_W = 1, FIELD_W = 75;

// A fake display: the host's frame loop asks `requestFrame` for each frame,
// so the test owns the clock. `tickAll()` draws one frame (1/60 s later);
// `frames.size` is 1 while the game loop runs.
const frames = new Map();
let nextHandle = 1;
let clock = 1000;
const requestFrame = (fn) => {
  const h = nextHandle++;
  frames.set(h, fn);
  return h;
};
const cancelFrame = (h) => frames.delete(h);
const tickAll = () => {
  clock += 1000 / 60;
  const due = [...frames.values()];
  frames.clear();
  for (const fn of due) fn(clock);
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
await app.load(core, appWasm, { log: () => {}, manifestText, requestFrame, cancelFrame });
// A fixed `Math.random` seed (core 1.12): the same game each time.
app.init([{ tag: INIT_RANDOM_SEED, data: new Uint8Array([1, 0, 0, 0, 0, 0, 0, 0]) }]);

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
// The paddles and the ball are absolute boxes in the court (UI API 1.9):
// their insets are their positions.
const inset = (n, prop) => n.props.get(prop) ?? 0;
const paddleTop = (label) => inset(labelled(label), Prop.top);
/** The ball's field position (the court has the left paddle column first). */
const ball = () => {
  const b = labelled("Ball");
  return { x: inset(b, Prop.left) - PADDLE_W, y: inset(b, Prop.top) };
};
/** A key on the court (UI API 1.9 onKeyDown/onKeyUp). */
const courtKey = (eventCode, key) => {
  const c = nodes().find((n) => String(n.props.get(Prop.label) ?? "").startsWith("Court"));
  assert.ok(c, "the court");
  app.onEvent({ kind: "ui", handler: c.listeners.get(eventCode), event: eventCode, value: key });
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
// The ball starts in the middle of the court (fractional units, UI API 1.11).
assert.deepEqual(start, { x: (FIELD_W - BALL) / 2, y: (FIELD_H - BALL) / 2 });
assert.equal(frames.size, 0, "no game loop before Start");

// -- Start: the ball moves toward the computer ---------------------------------
press("Start");
assert.equal(frames.size, 1, "Start starts the game loop");
assert.ok(labelled("Pause"), "the start button now pauses");
for (let i = 0; i < 10; i++) tickAll();
const moved = ball();
assert.ok(moved.x > start.x, `the ball moves right: ${JSON.stringify([start, moved])}`);
assert.notEqual(moved.y, start.y, "the ball also moves up or down");
// The ball moves the same distance on each frame (whole units gave steps of 1, ..., 0).
const xs = [ball().x];
for (let i = 0; i < 5; i++) {
  tickAll();
  xs.push(ball().x);
}
const steps = xs.slice(1).map((x, i) => x - xs[i]);
assert.ok(steps.every((s) => Math.abs(s - 0.9) < 1e-6), `even steps of 0.9 units: ${steps}`);

// Pause stops the loop; Resume starts it again.
press("Pause");
assert.equal(frames.size, 0);
const paused = ball();
tickAll();
assert.deepEqual(ball(), paused);
press("Resume");
assert.equal(frames.size, 1);
// Core 1.12: the game pauses when the window goes to the background (`isActive`).
app.onEvent({ kind: "lifecycle", lifecycleKind: 1 });
assert.equal(frames.size, 0, "the background pauses the game");
assert.ok(texts().includes("Paused"));
app.onEvent({ kind: "lifecycle", lifecycleKind: 0 });
assert.equal(frames.size, 0, "the game stays paused in the foreground");
press("Resume");
assert.equal(frames.size, 1);

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
// Keys: a held arrow key moves the paddle until it is released.
courtKey(Event.keyDown, "ArrowUp");
for (let i = 0; i < 3; i++) tickAll();
const held = paddleTop("Your paddle");
assert.ok(held < FIELD_H - PADDLE_H, `ArrowUp moves the paddle: ${held}`);
courtKey(Event.keyUp, "ArrowUp");
tickAll();
assert.equal(paddleTop("Your paddle"), held, "releasing the key stops the paddle");
courtKey(Event.keyDown, "s");
tickAll();
assert.ok(paddleTop("Your paddle") > held, "S moves the paddle down");
courtKey(Event.keyUp, "s");
// UI API 1.12: a drag on the court aims the paddle; the value [x, y] gives
// the callback two arguments (core 1.12).
const courtPointer = (eventCode, x, y) => {
  const c = nodes().find((n) => String(n.props.get(Prop.label) ?? "").startsWith("Court"));
  app.onEvent({ kind: "ui", handler: c.listeners.get(eventCode), event: eventCode, value: [x, y] });
};
courtPointer(Event.pointerDown, 30, 10);
for (let i = 0; i < 40; i++) tickAll();
assert.equal(paddleTop("Your paddle"), 5, "the paddle center goes to the pointer");
courtPointer(Event.pointerMove, 30, 30.5);
for (let i = 0; i < 30; i++) tickAll();
assert.ok(Math.abs(paddleTop("Your paddle") - 25.5) < 1e-6, `the paddle follows the drag: ${paddleTop("Your paddle")}`);
courtPointer(Event.pointerUp, 30, 30.5);
courtPointer(Event.pointerMove, 30, 2);
for (let i = 0; i < 10; i++) tickAll();
assert.ok(Math.abs(paddleTop("Your paddle") - 25.5) < 1e-6, "a move without a button down does not move the paddle");

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
while ((!wallBounce || !computerReturn || points() === 0) && ticks < 3000 && frames.size > 0) {
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
for (let i = 0; i < 20000 && frames.size > 0; i++) tickAll();
assert.equal(frames.size, 0, "the loop stops when the game is over");
const [you, cpu] = scores();
assert.ok(you === 5 || cpu === 5, `one side has 5 points: ${you}:${cpu}`);
assert.ok(texts().includes(cpu === 5 ? "The computer wins." : "You win!"));
press("Play again");
assert.deepEqual(scores(), [0, 0]);
assert.equal(frames.size, 1);

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
