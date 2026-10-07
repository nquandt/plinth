# Guide: keep your app fast

This guide tells you how to keep a Plinth app fast and inside its limits. It is for app authors. The measurements are in `docs/GAPS.md` ("Desktop frame cost", "Games", "Async code size").

## 1. Keep layouts flat

On the desktop, each nested flex box about doubles the layout work of its subtree (`docs/GAPS.md` F1). The cause is in the layout engine of `gpui-ce`, and a fix is open (`docs/PRODUCTION.md` R7). Until then:

- Do not put a `Box` in a `Box` in a `Box` when one box can do the work. Use `gap`, `padding`, `align` and `justify` on one box.
- Do not wrap a single child in a `Box` only for spacing.

The cost does not follow the number of nodes. In a release build, `settings-gallery` took 7.1 ms for one frame with 26 nodes, and `big-list` took 1.6 ms with 21,008 nodes.

## 2. Long lists

- Use `List` (or `Grid`) for a long collection. `List` is virtualized and keyed (`docs/ui.md` "Collections"). Give each item a stable `key`.
- In a column `Scroll`, the desktop builds only the children near the viewport when there are 40 or more children that each have a fixed `height`. In a horizontal `Scroll`, it builds only the visible children when there are 16 or more children with a fixed `width`. Give the rows of a long table a `height` and its cells a `width` (`examples/7guis/cells`).

## 3. Changes are cheap

The runtime remembers the last value of each prop and each text, and sends no change when the value is the same (`docs/GAPS.md` G8). You can set a signal to the same value with no cost to the host.

## 4. Motion with `onFrame`

For motion and games, use `onFrame` (core 1.12), not `setInterval`:

- `onFrame(callback)` calls `callback(dt)` before each frame that the host draws. `dt` is the time since the previous frame, in milliseconds (0 for the first frame). Move by `speed * dt`, so the speed is the same at each frame rate.
- Limit `dt`. The hosts draw no frames while the window is hidden or minimized, so the first `dt` after that can be large. `examples/pong` takes at most 50 ms in one step.
- Stop the loop with `cancelFrame(id)` when the motion ends. While a frame timer runs, the desktop host draws frames with no pause.
- Stop the loop when the app goes to the background (`isActive()`) and when the screen goes away (`onCleanup`).

This example is checked with `plinth check` against `std/time.d.ts` and `std/ui.d.ts`:

```tsx
import { app, signal, effect, onCleanup, isActive, Screen, Section, Box, Button, List, Row } from "plinth:ui";
import { onFrame, cancelFrame } from "plinth:time";
import { Math } from "plinth:core";

/** Spacing units per millisecond. */
const SPEED = 0.02;
/** The longest step: a frame after a hidden window counts as this. */
const MAX_DT = 50;

interface Item {
  id: number;
  title: string;
}

const items: Item[] = [];
for (let i = 0; i < 10000; i++) {
  items.push({ id: i, title: "Item " + i });
}

function Home() {
  const x = signal(0);
  const loop = signal<number | null>(null);

  const stop = () => {
    const id = loop();
    if (id !== null) {
      cancelFrame(id);
      loop.set(null);
    }
  };
  const start = () => {
    if (loop() === null) {
      loop.set(onFrame((dt) => x.set((x() + SPEED * Math.min(dt, MAX_DT)) % 60)));
    }
  };

  // Stop the loop in the background and when the screen goes away.
  effect(() => {
    if (!isActive()) {
      stop();
    }
  });
  onCleanup(stop);

  return (
    <Screen title="Motion">
      <Section title="Motion">
        <Box height={6} bg="surface" radius="sm">
          <Box position="absolute" left={x()} top={1} width={4} height={4} radius="full" bg="accent" />
        </Box>
        <Button label={loop() === null ? "Start" : "Stop"} onPress={() => (loop() === null ? start() : stop())} />
      </Section>
      <Section title="Items">
        <List items={items} key={(it) => it.id} row={(it) => <Row title={it.title} />} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Motion", component: Home } } });
```

Insets and sizes can be fractional spacing units (UI API 1.11), so a small step each frame gives smooth motion. `examples/pong` is a complete game on `onFrame`.

## 5. Measure a release build

- `plinth dev` adds hot reload code to your app. `plinth build` does not. Measure the package that `plinth build` makes, with `plinth run dist/<name>.plnt`.
- Use a release build of the `plinth` CLI (`cargo build --release -p plinth-cli`, `docs/getting-started.md` §1). A debug build of the host is much slower: for example, five `Picker`s took 2.8 ms for one frame in a release build and about 16 ms in a debug build (`docs/GAPS.md` F1).
- In a debug build of the host, `PLINTH_TRACE_RENDER=1` writes the render time of each frame to the console.

## 6. The limits

The desktop host stops an app that goes over a limit. It shows "This app stopped" with a plain reason.

| Limit | Desktop | Web |
|---|---|---|
| Time of one call (`init`, or one event with its handlers) | 2 s. Reason: "the app is not responding (time limit reached)". | No limit yet. An endless loop freezes the browser tab (`docs/PRODUCTION.md` R3). |
| Memory | 64 MiB. Reason: "out of memory". | No limit yet. |
| Stack | A limit. Reason: "too many nested calls (stack overflow)". | The browser's limit. |

On the wasmi engine (iOS, and `PLINTH_ENGINE=wasmi` on the desktop), the time limit uses fuel, not the clock. A heavy computation can reach the limit sooner than on wasmtime (`docs/GAPS.md` "wasmi engine").

To stay inside the time limit:

- Do long work in parts. For example, process a part of the data in each `setTimeout` callback.
- Do not run a large computation at start (in `init`).

## 7. App size

The `.plnt` file holds only your app code. The counter example is about 2 KB. Size is a goal, not a limit. `async` code costs about 1.35 KB one time in an app, and about 320 B for each `await`. A callback costs about 80 B. In a small app where size is important, the callback form (for example `confirm(message, done)`) is smaller than `await`.
