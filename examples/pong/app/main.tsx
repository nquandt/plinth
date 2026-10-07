// Pong, built from the Level 2 primitives (UI API 1.6). The rules live in
// game.ts; this file only draws the court and runs the game loop.
//
// Plinth has no absolute positioning and no Canvas yet, so the court is
// a row of fixed-size columns (paddle, left half, net, right half,
// paddle), and each moving part is placed by an empty spacer Box above
// it (and, for the ball, beside it). The ball is drawn in the half of
// the field that it is in. All sizes are spacing units (4 px).
// There are no key events for apps, so the paddle moves with Pressable
// buttons: "Up" and "Down" set its direction until "Stop" or the wall.
import { app, signal, computed, Screen, Section, Box, Span, Pressable } from "plinth:ui";
import { setInterval, clearInterval, monotonicNow } from "plinth:time";
import { Math } from "plinth:core";
import {
  Game,
  newGame,
  step,
  toggle,
  cell,
  FIELD_W,
  FIELD_H,
  PADDLE_W,
  PADDLE_H,
  BALL,
  NET_W,
  HALF_W,
  WIN_SCORE,
} from "./game";

/** The game loop interval: about 60 ticks per second. */
const TICK_MS = 16;
/** The dashes of the net. */
const DASHES: number[] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

function Pong() {
  const game = signal<Game>(newGame());
  const move = signal(0);
  const timer = signal<number | null>(null);
  // Measured tick rate (ticks per second over the last second).
  const rate = signal(0);
  const rateStart = signal(0);
  const rateTicks = signal(0);

  const ballX = computed(() => cell(game().ballX));
  const ballY = computed(() => cell(game().ballY));
  // The half that draws the ball: the left half for x < HALF_W, else the right half.
  const inLeft = computed(() => ballX() + BALL / 2 < HALF_W + NET_W / 2);
  const leftBallX = computed(() => (inLeft() ? Math.min(ballX(), HALF_W - BALL) : 0));
  const rightBallX = computed(() => (inLeft() ? 0 : Math.max(0, Math.min(ballX() - HALF_W - NET_W, HALF_W - BALL))));

  const stopLoop = () => {
    const id = timer();
    if (id !== null) {
      clearInterval(id);
      timer.set(null);
    }
  };

  const tick = () => {
    const next = step(game(), move());
    game.set(next);
    if (next.leftY <= 0 || next.leftY >= FIELD_H - PADDLE_H) {
      move.set(0);
    }
    if (next.phase !== "playing") {
      stopLoop();
    }
    const now = monotonicNow();
    rateTicks.set(rateTicks() + 1);
    if (now - rateStart() >= 1000) {
      rate.set(Math.round((rateTicks() * 1000) / (now - rateStart())));
      rateStart.set(now);
      rateTicks.set(0);
    }
  };

  const startStop = () => {
    const next = toggle(game());
    game.set(next);
    if (next.phase === "playing") {
      if (timer() === null) {
        rateStart.set(monotonicNow());
        rateTicks.set(0);
        timer.set(setInterval(tick, TICK_MS));
      }
    } else {
      stopLoop();
    }
  };

  const status = computed(() => {
    const g = game();
    if (g.phase === "ready") {
      return `First to ${WIN_SCORE} wins. Press Start.`;
    }
    if (g.phase === "paused") {
      return "Paused";
    }
    if (g.phase === "over") {
      return g.winner === "left" ? "You win!" : "The computer wins.";
    }
    return rate() > 0 ? `Rally ${g.rally} · ${rate()} ticks/s` : `Rally ${g.rally}`;
  });
  const startLabel = computed(() => {
    const p = game().phase;
    return p === "ready" ? "Start" : p === "playing" ? "Pause" : p === "paused" ? "Resume" : "Play again";
  });

  return (
    <Screen title="Pong">
      <Section>
        <Box align="center" gap={3}>
          <Box direction="row" justify="between" align="center" width={FIELD_W + 2 * PADDLE_W}>
            <Box gap={1}>
              <Span size="sm" fg="text.muted">You</Span>
              <Span size="2xl" weight="bold" mono={true}>{`${game().leftScore}`}</Span>
            </Box>
            <Box gap={1} align="end">
              <Span size="sm" fg="text.muted">Computer</Span>
              <Span size="2xl" weight="bold" mono={true}>{`${game().rightScore}`}</Span>
            </Box>
          </Box>
          <Box label="Court" direction="row" width={FIELD_W + 2 * PADDLE_W} height={FIELD_H} bg="text" radius="sm">
            <Box width={PADDLE_W} height={FIELD_H}>
              <Box height={cell(game().leftY)} />
              <Box label="Your paddle" width={PADDLE_W} height={PADDLE_H} bg="background" />
            </Box>
            <Box width={HALF_W} height={FIELD_H}>
              <Box height={inLeft() ? ballY() : 0} />
              <Box direction="row" height={BALL}>
                <Box width={leftBallX()} />
                <Box label="Ball (left half)" width={BALL} height={BALL} bg={inLeft() ? "background" : "none"} />
              </Box>
            </Box>
            <Box label="Net" width={NET_W} height={FIELD_H} justify="between" paddingY={1}>
              {DASHES.map((d) => (
                <Box width={NET_W} height={2} bg="text.muted" />
              ))}
            </Box>
            <Box width={HALF_W} height={FIELD_H}>
              <Box height={inLeft() ? 0 : ballY()} />
              <Box direction="row" height={BALL}>
                <Box width={rightBallX()} />
                <Box label="Ball (right half)" width={BALL} height={BALL} bg={inLeft() ? "none" : "background"} />
              </Box>
            </Box>
            <Box width={PADDLE_W} height={FIELD_H}>
              <Box height={cell(game().rightY)} />
              <Box label="Computer paddle" width={PADDLE_W} height={PADDLE_H} bg="background" />
            </Box>
          </Box>
          <Span fg="text.muted">{status()}</Span>
          <Box direction="row" gap={2} align="center" width={FIELD_W + 2 * PADDLE_W}>
            <Pressable label="Up" role="button" paddingX={4} paddingY={2} radius="md" border="border"
                       bg={move() < 0 ? "selected" : "surface"} onPress={() => move.set(-1)}>
              <Span weight="medium">Up</Span>
            </Pressable>
            <Pressable label="Stop" role="button" paddingX={4} paddingY={2} radius="md" border="border"
                       bg="surface" onPress={() => move.set(0)}>
              <Span weight="medium">Stop</Span>
            </Pressable>
            <Pressable label="Down" role="button" paddingX={4} paddingY={2} radius="md" border="border"
                       bg={move() > 0 ? "selected" : "surface"} onPress={() => move.set(1)}>
              <Span weight="medium">Down</Span>
            </Pressable>
            <Box grow={1} />
            <Pressable label={startLabel()} role="button" paddingX={4} paddingY={2} radius="md" bg="accent" onPress={startStop}>
              <Span fg="on.accent" weight="semibold">{startLabel()}</Span>
            </Pressable>
          </Box>
        </Box>
      </Section>
    </Screen>
  );
}

export default app({
  accent: "green",
  screens: {
    pong: { title: "Pong", icon: "star", component: Pong },
  },
});
