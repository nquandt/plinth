# Validation plan: apps that test Plinth

This plan uses real apps to find where Plinth's UX and architecture hold up and where they fail. It has three phases, done in this sequence:

1. **V1: 7GUIs.** Seven small, standard tasks that test a UI framework. Other frameworks have published implementations, so we can compare.
2. **V2: one flagship app.** A notes app that pushes the UX: text editing, navigation, search, persistence and shortcuts.
3. **V3: an app suite.** Several apps that run together in one host. The tests look at the boundaries between apps, and between each app and the host.

Each task below names what it tests, the Plinth features it uses, the known gaps it will hit, and its exit criteria. A gap is not a reason to skip a task. Write the gap down in `docs/GAPS.md`, then close it or record a decision.

SPEC.md §15 (milestone M9) links to this plan.

## 1. Rules for every validation app

- The app is in `examples/7guis/<name>` (V1) or `examples/<name>` (V2, V3). It must be valid TypeScript for `tsc`, like every example.
- The app runs on the desktop host and the web host. A difference between the two hosts is a defect, unless the docs say otherwise.
- Each app has headless tests: `crates/plinth-compiler/tests/` drives the app in wasmtime, `crates/plinth-shoot` takes screenshots, and `web/test/` covers the web host. Interaction tests use real input events where it matters (keys, a held mouse button), as `web/test/run-a11y.mjs` does.
- axe finds no serious or critical problems on the web. The desktop AccessKit tree names every control.
- Record these numbers for each app in §6: lines of app code, `.plnt` size, start time to the first frame, and memory for each instance.

## 2. V1: 7GUIs

Reference: the 7GUIs benchmark by Eugen Kiss (<https://eugenkiss.github.io/7guis/>).

| # | Task | What it tests | Plinth features | Known gaps to check |
|---|---|---|---|---|
| 1 | Counter | State and events | `signal`, `Button`, `Text` | None. `examples/counter` already does it; copy it into the suite. |
| 2 | Temperature converter | Two-way data flow; input validation | Two `TextField`s with `value` and `onChange`; a parse that can fail | Do two fields that update each other cause a loop? Does invalid input stay in the field without a reset? (The web two-way binding defect of 2026-10-06 was in this area.) |
| 3 | Flight booker | Constraints between fields; enable and disable | `Picker`, `TextField` or `DatePicker`, `Button disabled`, `tone="danger"` for invalid input | Date parsing and comparison (`plinth:time`). A field that shows an invalid state. |
| 4 | Timer | Concurrency; live updates; responsiveness while work runs | `setInterval`, `Progress`, `Slider` | `examples/timer` covers part of it. The UI must stay responsive while the timer ticks. The web click defect of 2026-10-06 was here: test with held clicks. |
| 5 | CRUD | Lists, filter, selection, model separate from view | `List`, `Row`, `TextField` filter, selection state, `Map` | Row selection as a list prop. `examples/contacts` and `examples/budget` cover part of it. |
| 6 | Circle drawer | Custom drawing, context menu, dialog, undo and redo | Needs `Canvas` (SPEC.md §6.3, `ui.canvas`), `Menu` as a context menu, `Dialog` with a `Slider` | **`Canvas` is not built.** A context menu at a pointer position. Undo and redo are app code; check that the language makes them easy (classes, arrays of commands). |
| 7 | Cells | Formula parsing, dependency tracking, change propagation, a large grid | `Grid`, `TextField` in cells, `Map`, recursion, `computed` | Grid virtualization (today only `List` virtualizes, above 200 rows). Edit a cell in place. The cost of many signals (26 × 100 cells). Cycle detection in the app. |

**Exit criteria for V1:**

- All seven apps run on the desktop and on the web, with tests.
- Cells: one edit updates only the dependent cells. A scroll through 26 × 100 cells stays under 16 ms for each frame on the desktop (measure with `PLINTH_TRACE_RENDER=1`).
- Circle drawer: `Canvas` exists as a control with a capability, on both hosts (web: a `<canvas>` element, see SPEC.md M5).
- A comparison table in §6 with lines of code against two published implementations (for example React and Svelte).

## 3. V2: flagship notes app

The flagship app is a notes app, in the style of a small Obsidian or Notion. It replaces or extends `examples/notes`. If Plinth handles a text editor well, it handles most other apps.

| Area | What to build | Known gaps to check |
|---|---|---|
| Text editing | Cursor, selection, IME, clipboard, undo, in a long text | `TextArea` is plain text. Rich text (bold, headings, lists, links) needs a design: a `RichText` control with a document model, or Markdown with a preview. This is the largest design question in V2. |
| Navigation | Nested notes (folders or links), back and forward, deep links | Stack navigation exists. Arguments for `navigate.push` (SPEC.md M2 status). `plinth://` links into a note. |
| Search | Full-text search that updates while the user types | Performance of string search in the runtime on a few thousand notes. |
| Reorder | Drag and drop of notes and blocks | **No drag and drop in the UI API.** Design it as intent (a `reorderable` list), not as pixels. |
| Persistence | Offline storage; state survives a restart | `store.kv` only. Size limits; a `store.sql` capability (SPEC.md §11) may be necessary. |
| Shortcuts | Keyboard shortcuts for commands | `Action.shortcut` is not built (SPEC.md §6.3). |

**Exit criteria for V2:** a person can write, find and organize 1,000 notes for a day without a defect that loses data. Typing in a 50 KB note has no visible delay on the desktop or the web. Every gap above has a decision in `docs/GAPS.md` or in the SPEC.

## 4. V3: an app suite and isolate tests

Build a second and a third app (for example a files app and a media player) to run together with the notes app in one host. The most important tests are not inside one app. They are between apps, and between each app and the host.

| Test | What to check | Current state |
|---|---|---|
| Lifecycle | Suspend, resume, kill and restore an app. State survives, and the UI comes back at the same screen and scroll position. | Not built. SPEC.md §18 asks that a host can run, suspend and drop more than one app. Hot reload already moves signal state from one instance to a new one; restore can use the same mechanism. |
| Crash isolation | One app traps, loops forever, or uses too much memory. The host and the other apps stay responsive. | The runner has a time limit for each event (epoch interruption) and a memory cap (SPEC.md §9). Test it with several apps in one host (Hub H3 runs several apps in one process). The web host: each app in its own iframe. |
| Inter-app communication | Share a file or a note from one app to another; drag and drop between apps; a shared clipboard. | Not designed. Sharing needs an intent: the user picks the target, and the host moves the data, never a path (like `fs.pick` tokens). |
| Resource fairness | A heavy loop in one app does not make the others stutter. | Check how the desktop host schedules several guests (one thread or more). The time limit for each event is a start, not a full answer for long work. |
| Permissions and sandbox | An app reaches only what it was granted. | Capability checks at link time, grants and consent (docs/HUB.md). Add tests that try each denied call from a running app, in each host. |
| Start time and memory | Time to the first frame and memory for each instance. Users feel these numbers. | Measure. Goals: to be set after the first measurements; record them in §6. |

**Exit criteria for V3:** three apps run together in one desktop host and in the web Hub. Each test above has an automated test or a written decision. A crash or a hang in one app never stops another app.

## 5. Sequence and size

1. V1 tasks 1–5 (small; they find defects in the current controls).
2. V1 task 7, Cells (performance and many signals).
3. `Canvas`, then V1 task 6, Circle drawer.
4. V2, the notes app, after a decision on rich text.
5. A second app, then V3.

## 6. Results

Fill in this section as the apps are done.

| App | Lines of app code | `.plnt` size | First frame (desktop) | First frame (web) | Memory | Notes |
|---|---|---|---|---|---|---|
| | | | | | | |
