// Pong, built from the Level 2 primitives. The rules live in game.ts; this
// file only draws the court and runs the game loop.
//
// The court is one box; the paddles, the net and the ball are absolute
// boxes in it (UI API 1.9), placed by their insets. All sizes are spacing
// units (4 px); the insets are fractional units (UI API 1.11), so the ball
// moves the same distance on each tick. Keys: hold the arrow keys or W/S on the court to move the
// paddle, Space starts and pauses (UI API 1.9 onKeyDown/onKeyUp). Drag on
// the court with a mouse or a finger to move the paddle (UI API 1.12). The
// buttons below the court do the same for touch and pointer users.
import { app, signal, computed, Screen, Section, Box, Span, Pressable } from "plinth:ui";
import { onFrame, cancelFrame, monotonicNow } from "plinth:time";
import { Math } from "plinth:core";
import {
  Game,
  newGame,
  step,
  toggle,
  FIELD_W,
  FIELD_H,
  PADDLE_W,
  PADDLE_H,
  BALL,
  NET_W,
  HALF_W,
  WIN_SCORE,
  PLAYER_SPEED,
} from "./game";

/** One game tick: 1/60 s. The loop runs once per display frame (onFrame,
 * core 1.12) and steps the game by the frame time in ticks. */
const TICK_MS = 1000 / 60;
/** The longest frame time that the game takes in one step (a frame after a
 * pause or a hidden window counts as this). */
const MAX_FRAME_MS = 50;
/** The dashes of the net. */
const DASHES: number[] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

function Pong() {
  const game = signal<Game>(newGame());
  const move = signal(0);
  // UI API 1.12: while a pointer (a mouse button or a finger) is down on the
  // court, the paddle goes toward it at its normal speed.
  const aim = signal<number | null>(null);
  const timer = signal<number | null>(null);
  // Measured frame rate (frames per second over the last second).
  const rate = signal(0);
  const rateStart = signal(0);
  const rateTicks = signal(0);

  const stopLoop = () => {
    const id = timer();
    if (id !== null) {
      cancelFrame(id);
      timer.set(null);
    }
  };

  const frame = (dt: number) => {
    const k = Math.min(dt, MAX_FRAME_MS) / TICK_MS;
    const target = aim();
    // The pointer gives a part of a full move near the target, so the paddle stops on it.
    const dir = target === null ? move() : k > 0 ? Math.max(-1, Math.min(1, (target - game().leftY) / (PLAYER_SPEED * k))) : 0;
    const next = step(game(), dir, k);
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
        timer.set(onFrame(frame));
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
    return rate() > 0 ? `Rally ${g.rally} · ${rate()} frames/s` : `Rally ${g.rally}`;
  });
  // Keys on the court: a held key moves the paddle until it is released.
  const keyDown = (key: string) => {
    if (key === "ArrowUp" || key === "w") {
      move.set(-1);
    } else if (key === "ArrowDown" || key === "s") {
      move.set(1);
    } else if (key === "Space") {
      startStop();
    }
  };
  const pointer = (x: number, y: number) => {
    aim.set(Math.max(0, Math.min(FIELD_H - PADDLE_H, y - PADDLE_H / 2)));
  };
  const keyUp = (key: string) => {
    if ((key === "ArrowUp" || key === "w") && move() < 0) {
      move.set(0);
    } else if ((key === "ArrowDown" || key === "s") && move() > 0) {
      move.set(0);
    }
  };

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
          <Box
            label="Court: hold the arrow keys or W and S to move, Space to start or pause"
            width={FIELD_W + 2 * PADDLE_W}
            height={FIELD_H}
            bg="text"
            radius="sm"
            focus={{ border: "accent" }}
            onKeyDown={keyDown}
            onKeyUp={keyUp}
            onPointerDown={pointer}
            onPointerMove={(x, y) => {
              if (aim() !== null) {
                pointer(x, y);
              }
            }}
            onPointerUp={() => aim.set(null)}
          >
            <Box label="Net" position="absolute" left={PADDLE_W + HALF_W} top={0} width={NET_W} height={FIELD_H} justify="between" paddingY={1}>
              {DASHES.map((d) => (
                <Box width={NET_W} height={2} bg="text.muted" />
              ))}
            </Box>
            <Box label="Your paddle" position="absolute" left={0} top={game().leftY} width={PADDLE_W} height={PADDLE_H} bg="background" />
            <Box label="Computer paddle" position="absolute" right={0} top={game().rightY} width={PADDLE_W} height={PADDLE_H} bg="background" />
            <Box label="Ball" position="absolute" left={PADDLE_W + game().ballX} top={game().ballY} width={BALL} height={BALL} bg="background" radius="full" />
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
