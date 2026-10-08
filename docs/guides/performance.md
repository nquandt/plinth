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
- Measure the package that `plinth build --release` makes (§8): it has the app code that you ship.
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
- Use a `Map` or a `Set` to find items by a key. `get`, `set`, `has` and `delete` take the same time for 10 keys and for 10,000 keys. A search in an array (`find`, `indexOf`, `includes`) looks at each item.
- `sort` is a merge sort: a sort of 10,000 numbers takes about 5 ms in a release build.
- To make a long text in a loop, push the parts into an array and call `join` one time. The garbage collector does not run during a call, so `text = text + part` in a loop keeps a copy of each step in memory until the call ends.
- Do not run a large computation at start (in `init`).

## 7. App size

The `.plnt` file holds only your app code. The counter example is about 2 KB. Size is a goal, not a limit. `async` code costs about 1.35 KB one time in an app, and about 320 B for each `await`. A callback costs about 80 B. In a small app where size is important, the callback form (for example `confirm(message, done)`) is smaller than `await`.

## 8. Release builds

`plinth build --release` makes the app code smaller. It runs the Binaryen optimizer (`wasm-opt -Oz`) on the app module before it writes the package. Binaryen is built into `plinth`: you do not install another tool. `--release` also works with `--target web` and with `plinth native <dir>`.

```sh
plinth build --release              # dist/<name>.plnt, optimized
plinth build --release --target web # dist/web/, optimized
```

- Use `--release` for the package that you publish. Use `plinth build` (no flag) and `plinth dev` while you work: they are faster.
- The same source gives the same bytes each time. A signature and a digest stay the same from one release build to the next.
- The optimized app behaves the same. `plinth` checks the result: the same imports, the same `plinth-core` section, no exports, and a link into the core. If a check fails, `plinth` writes a warning and keeps the module that the compiler made.
- The build prints the size before and after: `built dist/counter.plnt in 175 ms (app 1674 B, 1939 B before wasm-opt, package 2 KiB)`.

Measurements over the 26 example apps (2026-10-07, Binaryen 116):

| | Debug build | `--release` |
|---|---|---|
| All app modules | 160,745 B | 138,863 B (-13.6 %) |
| All app modules, gzip | 69,821 B | 61,809 B (-11.5 %) |
| `counter` | 1,939 B | 1,674 B |
| `hub` (the largest) | 24,264 B | 21,070 B |

The speed does not change in a measurable way: most of the work is in the core, not in the app code. `-O2` and `-O3` gave larger modules than `-Oz` (-12.8 % and -12.2 %), and Binaryen 133 gave the same sizes as Binaryen 116 to within 40 B in total.

The core is not optimized by `--release`: the host supplies it, and a change of its bytes is a new core build. `-Oz` makes the 1.12 core 14 % smaller (131,188 B to 112,696 B; gzip 50,970 B to 48,452 B). Core 1.12 is shipped and frozen, so this step is planned for core 1.13 (`docs/PRODUCTION.md` T7).

## 9. Performance budgets (for Plinth contributors)

The gate makes sure that a change to Plinth does not make the examples larger or slower without a decision (`docs/PRODUCTION.md` R6). A test fails when a value goes over its budget. The failure message gives the metric, the budget, the value, and the command that writes new budgets.

| Metric | What is measured | Budget | Test | Runs in |
|---|---|---|---|---|
| `app_size` | `app.wasm` of each example (debug build, no `--release`), bytes | Measured value + 10 % | `crates/plinth-compiler/tests/budgets.rs` | `ci-local.sh full` and `nogui` |
| `memory` | Guest linear memory after `init`, and after 300 fixed UI events, for `counter`, `todo`, `budget`, `hub`, `big-list` | Measured value + 10 %, up to a full 64 KiB page | same | same |
| `start_ms` | `Runner::load` (the engine compiles the core and the app) and `init` to the first commit, separately, for the same five examples, debug build | 3 x the median at bless time; at least 100 ms (load) and 10 ms (init) | same | same |
| `core_size` | The core: raw (`target/rt`) and linked | No budget: the runtime has no size limit (SPEC.md §5.5). A change of more than 10 % prints a note. | same | same |
| `frame_ms` | Median `Window::draw` time of each example at 1000 x 800, debug build | 3 x the median at bless time; at least 20 ms | `crates/plinth-shoot/tests/frame_cost.rs` | `ci-local.sh full --shoot` |

The budgets are in `crates/plinth-compiler/tests/budgets.toml` and `crates/plinth-shoot/tests/frame_budgets.toml`. Do not edit these files by hand.

Sizes and memory are deterministic: the same source gives the same value each time. Their budgets are tight. Timings change from run to run, and other builds on the same machine make them slower. The timing tests:

- take 5 rounds that go through the apps in turn (interleaved), and use the median, not one sample;
- run the tests of `budgets.rs` one at a time, so that a compile does not slow a sample;
- use a budget of 3 times the median and a lowest budget. A timing budget finds a large regression (for example, an app that builds its UI two times), not a small one.

The timing budgets are absolute times on the machine that ran bless. A much slower machine can need new budgets. On a GitHub-hosted runner (the macOS CI job, `RUNNER_ENVIRONMENT=github-hosted`), the test prints the start times and does not check them. The size and memory budgets apply on all machines. A release build of the tests (`cargo test --release`) is faster and passes the debug budgets. The frame budgets apply only to a debug build.

### Update a budget

When a change makes a value larger and you agree with the change:

```sh
PLINTH_BLESS=1 cargo test -p plinth-compiler --test budgets
PLINTH_BLESS=1 cargo test -p plinth-shoot --test frame_cost every_example
```

Bless writes new budgets from the values of this run and prints each change (`old -> new`). Read the diff, and commit the budget file with the change that caused it. Bless the timings on a machine that does no other builds at that time.

When a value is much smaller than its budget (a size or a memory value at less than 75 % of the budget that bless would write now), the test prints a note. Bless to make the budget tighter.

Measured on 2026-10-08 (Windows, debug build, 3 runs; the spread is the range of the medians):

| Example | `app.wasm` | Memory after init | After 300 events | Load | Init | Frame |
|---|---|---|---|---|---|---|
| `counter` | 1,939 B | 1,152 KiB | 1,152 KiB | 300–316 ms | 0.1 ms | 4.2–4.3 ms |
| `todo` | 5,157 B | 1,152 KiB | 1,216 KiB | 290–304 ms | 0.2 ms | 11.8 ms |
| `budget` | 15,124 B | 1,152 KiB | 1,216 KiB | 419–467 ms | 0.5–0.6 ms | 28.1 ms |
| `hub` | 24,264 B | 1,152 KiB | 1,344 KiB | 470–520 ms | 0.4 ms | 8.6 ms |
| `big-list` | 3,923 B | 22,720 KiB | 22,720 KiB | 296–315 ms | 74–76 ms | 21.6–22.2 ms |

The core: 131,188 B raw, 68,432 B linked. The slowest frame is `7guis/cells` (5,666 nodes): 83.1–83.8 ms.
