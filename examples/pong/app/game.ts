// The Pong rules, without any UI. All positions and sizes are spacing
// units (one unit is 4 px), the same units that the court uses in
// main.tsx; speeds are units per tick. The UI calls `step` once per timer
// tick with a fixed time step, so the game is deterministic (no random
// numbers: plinth:core has no Math.random, so the serve angles come from
// a fixed list).
import { Math } from "plinth:core";

/** The width of the field between the two paddles. */
export const FIELD_W = 75;
/** The height of the court. */
export const FIELD_H = 48;
export const PADDLE_W = 2;
export const PADDLE_H = 10;
export const BALL = 2;
/** The width of the net in the middle of the field. */
export const NET_W = 1;
/** The width of each half of the field, at each side of the net: (FIELD_W - NET_W) / 2. */
// The whole court (two paddles and the field) is 79 units, 316 px: it fits a
// compact (phone) window. There is no way to scale it to the window.
export const HALF_W = 37;
/** The score that wins the game. */
export const WIN_SCORE = 5;

const SERVE_SPEED = 0.9;
const MAX_SPEED = 1.8;
const SPEED_UP = 1.06;
const MAX_VY = 1.0;
const PLAYER_SPEED = 1.2;
const COMPUTER_SPEED = 0.62;
/** Ticks that the ball waits at the center before a serve. */
const SERVE_WAIT = 30;
const SERVE_ANGLES: number[] = [0.35, -0.55, 0.6, -0.3, 0.5, -0.45, 0.2, -0.65];

export type Phase = "ready" | "playing" | "paused" | "over";

export interface Game {
  phase: Phase;
  ballX: number;
  ballY: number;
  velX: number;
  velY: number;
  /** The top of the player's paddle (left). */
  leftY: number;
  /** The top of the computer's paddle (right). */
  rightY: number;
  leftScore: number;
  rightScore: number;
  /** Ticks to wait before the ball moves (after a point). */
  wait: number;
  /** Paddle hits since the last serve. */
  rally: number;
  /** Totals, for tests and for the status line. */
  wallHits: number;
  paddleHits: number;
  /** "left", "right" or "" (no winner yet). */
  winner: string;
}

/** The direction the player's paddle moves: -1 up, 0 still, 1 down. */
export type Move = number;

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

/** A new game, ready to start; the first serve goes to the computer. */
export function newGame(): Game {
  return {
    phase: "ready",
    ballX: (FIELD_W - BALL) / 2,
    ballY: (FIELD_H - BALL) / 2,
    velX: SERVE_SPEED,
    velY: SERVE_ANGLES[0],
    leftY: (FIELD_H - PADDLE_H) / 2,
    rightY: (FIELD_H - PADDLE_H) / 2,
    leftScore: 0,
    rightScore: 0,
    wait: 0,
    rally: 0,
    wallHits: 0,
    paddleHits: 0,
    winner: "",
  };
}

/** Starts, pauses or resumes. A finished game starts again. */
export function toggle(g: Game): Game {
  if (g.phase === "over") {
    const fresh = newGame();
    return { ...fresh, phase: "playing" };
  }
  if (g.phase === "playing") {
    return { ...g, phase: "paused" };
  }
  return { ...g, phase: "playing" };
}

/** Puts the ball at the center and serves toward `dir` (-1 left, 1 right). */
function serve(g: Game, dir: number): void {
  const n = g.leftScore + g.rightScore;
  g.ballX = (FIELD_W - BALL) / 2;
  g.ballY = (FIELD_H - BALL) / 2;
  g.velX = SERVE_SPEED * dir;
  g.velY = SERVE_ANGLES[n % SERVE_ANGLES.length];
  g.wait = SERVE_WAIT;
  g.rally = 0;
}

/** The new vertical speed after a hit at `ballY` on a paddle whose top is `paddleY`. */
function deflect(ballY: number, paddleY: number): number {
  const offset = (ballY + BALL / 2 - (paddleY + PADDLE_H / 2)) / (PADDLE_H / 2 + BALL / 2);
  return clamp(offset, -1, 1) * MAX_VY;
}

function hits(ballY: number, paddleY: number): boolean {
  return ballY + BALL > paddleY && ballY < paddleY + PADDLE_H;
}

/** Moves the computer's paddle toward the ball, at a limited speed. */
function computer(g: Game): void {
  // When the ball goes away, the computer goes back to the center.
  const target = g.velX > 0 ? g.ballY + BALL / 2 - PADDLE_H / 2 : (FIELD_H - PADDLE_H) / 2;
  const delta = clamp(target - g.rightY, -COMPUTER_SPEED, COMPUTER_SPEED);
  g.rightY = clamp(g.rightY + delta, 0, FIELD_H - PADDLE_H);
}

/** One tick of the game. `move` is the player's paddle direction. */
export function step(prev: Game, move: Move): Game {
  if (prev.phase !== "playing") {
    return prev;
  }
  const g: Game = { ...prev };
  g.leftY = clamp(g.leftY + move * PLAYER_SPEED, 0, FIELD_H - PADDLE_H);
  computer(g);
  if (g.wait > 0) {
    g.wait = g.wait - 1;
    return g;
  }

  g.ballX = g.ballX + g.velX;
  g.ballY = g.ballY + g.velY;

  // The top and bottom walls.
  if (g.ballY < 0) {
    g.ballY = -g.ballY;
    g.velY = -g.velY;
    g.wallHits = g.wallHits + 1;
  } else if (g.ballY > FIELD_H - BALL) {
    g.ballY = 2 * (FIELD_H - BALL) - g.ballY;
    g.velY = -g.velY;
    g.wallHits = g.wallHits + 1;
  }

  // The player's side (left).
  if (g.ballX <= 0) {
    if (hits(g.ballY, g.leftY)) {
      g.ballX = -g.ballX;
      g.velX = Math.min(MAX_SPEED, -g.velX * SPEED_UP);
      g.velY = deflect(g.ballY, g.leftY);
      g.rally = g.rally + 1;
      g.paddleHits = g.paddleHits + 1;
    } else {
      g.rightScore = g.rightScore + 1;
      score(g, 1);
    }
  } else if (g.ballX >= FIELD_W - BALL) {
    // The computer's side (right).
    if (hits(g.ballY, g.rightY)) {
      g.ballX = 2 * (FIELD_W - BALL) - g.ballX;
      g.velX = -Math.min(MAX_SPEED, g.velX * SPEED_UP);
      g.velY = deflect(g.ballY, g.rightY);
      g.rally = g.rally + 1;
      g.paddleHits = g.paddleHits + 1;
    } else {
      g.leftScore = g.leftScore + 1;
      score(g, -1);
    }
  }
  return g;
}

/** After a point: the game ends, or a new serve goes toward the side that won the point. */
function score(g: Game, serveDir: number): void {
  if (g.leftScore >= WIN_SCORE || g.rightScore >= WIN_SCORE) {
    g.phase = "over";
    g.winner = g.leftScore >= WIN_SCORE ? "left" : "right";
    g.ballX = (FIELD_W - BALL) / 2;
    g.ballY = (FIELD_H - BALL) / 2;
    return;
  }
  serve(g, serveDir);
}

/** The ball's whole-unit position, as the court draws it. */
export function cell(v: number): number {
  return Math.round(v);
}
