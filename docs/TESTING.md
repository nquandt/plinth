# Testing apps: `plinth test` and `plinth:test`

**Status (2026-10-08):** step 1 of PRODUCTION T3 is done. Tests run on the
desktop runner (wasmtime or wasmi, no window). Section 12 lists what is not
done.

An app author writes tests in Plinth TS, in the app, next to the code. The
tests use the module `plinth:test`. `plinth test` (or `npm test` in a project
from `plinth new` or `npm create plinth`) runs them.

```tsx
// app/tasks.test.tsx
import { test, render, expect } from "plinth:test";
import Tasks from "./tasks";

test("adds a task", async () => {
  const s = await render(<Tasks />);
  await s.type("Task", "Buy milk");     // the text field with the label "Task"
  await s.press("Add task");            // the button with the label "Add task"
  expect(s.row("Buy milk")).toExist();  // a list row with this title
});
```

```
$ plinth test
  ok  adds a task (3 ms)
1 passed, 0 failed
```

Examples: `examples/counter/app/counter.test.tsx`,
`examples/todo/app/tasks.test.tsx`, and the `plinth new` template.

## 1. Test files and the test build

- A test file is `app/**/*.test.ts` or `app/**/*.test.tsx`.
- Only a test file can import `plinth:test`. In any other file, the import
  is an error: "`plinth:test` is for test files only".
- `plinth test` makes a **test build**: the app (all the modules that
  `app/main.tsx` imports), the test files, and `plinth:test`. The compiler
  adds two screens after the screens of the app: the **view screen** (it
  shows what `render` gives) and the **harness screen** (the runner sends
  its answers there). The test build links into the release core. It is
  never written to a file.
- `plinth build`, `plinth check` and `plinth dev` start from
  `app/main.tsx` and read only the modules that it imports (the closed
  world, SPEC §4.1). An app module cannot import a test file, because a
  test file imports `plinth:test`. Thus a package never contains test code.
  `crates/plinth-cli/tests/plinth_test.rs` checks that the app module is the
  same with and without a test file.
- A compile error in a test file is a diagnostic, as in any other file, and
  `plinth test` stops with exit code 1.

## 2. Where the code runs (the design choice)

The test code runs **in the guest**: `plinth:test` is Plinth TS
(`crates/plinth-compiler/src/testkit.tsx`) that the compiler adds to the test
build. The UI tree is **in the host**: the core does not keep a tree (SPEC
§8.3), and the runner already has one (`plinth_ui::tree::Tree`).

The test code and the runner talk over two channels that every core has:

| Direction | Channel | What goes over it |
|---|---|---|
| Test to runner | `dev.log` (`console.log`), lines that start with `U+0001 plinth:test U+0001` | One record for each line: `test`, `render`, `renderApp`, `act` (press, type, ...), `expect`, `read`, `tick`, `settle`, `fail`, `done`, `error`. Fields are separated by U+001F. |
| Runner to test | a `change` event on the text field of the harness screen | `run <index>` starts a test; `ok <reply>` lets the test continue after a step. |

A step goes like this. The test calls `await s.press("Add")`. The guest sends
the record `act press ...` and gives a promise that waits. The guest call
ends, and the core flushes and commits as after any event. The runner
applies the commits to its tree, finds the node, and sends the `press` event
to the handler of the node through the normal event path (`on-event`). The
runner applies the commits of that event, and sends `ok` to the harness
field. The promise resolves, and the test continues.

**The core and the WIT do not change.** The kit adds no core function and
no WIT item: core 1.12 stays frozen, and core 1.13 and `app.wit` get nothing
for tests. A test build uses only `dev.log`, `ui.commit` and `on-event`, which
every core and every host have. Thus the same test build can later run on
wasmi and on the web host with no new contract (section 11).

The options that we did not take:

1. **A test interface in the WIT (`plinth:app/test`) with new core
   functions.** The WIT is append-only and ships in every core, so a test
   interface would be a contract for all hosts for ever. New core functions
   would go into 1.13 and every host would have to supply them.
2. **A separate test world and a test core** (a third `plinth-rt` build,
   like the dev core). This keeps the production WIT clean, but it needs a
   third core build, a second componentize path and a second set of host
   imports in each engine. The append linker cannot add imports to an app
   module (`plinth-link/src/link.rs`), so the test functions would have to be
   in the core.
3. **Synchronous queries from the guest.** The core commits its ops only
   at the end of a call, so the host tree is not current in the middle of a
   call. A synchronous `press` would need a new core function to flush and
   commit in the middle of a call, and a re-entrant call into the component,
   which the component model does not allow.

The cost of the choice: each step is a promise (`await s.press(...)`), as in
Testing Library `user-event` and Playwright. An expectation about the screen
does not return a value to the test (the runner checks it, section 6).

## 3. The API

`plinth:test` (typings: `std/test.d.ts`):

| Name | What it does |
|---|---|
| `test(name, async () => { ... })` | Registers a test. A test function is `async`. |
| `testOnly(name, async () => { ... })` | The same, but when the file has any `testOnly` or `describeOnly`, only those tests run (the plain `test`s do not). Plinth TS has no function with properties, so this replaces `test.only`. |
| `testSkip(name, async () => { ... })` | The same as `test`, but the test shows as "skip" in the output and does not run. Replaces `test.skip`. |
| `describe(name, () => { ... })` | Groups tests under `name`: the group name goes before the test name ("group > test") in the output and in `--filter`. `describe` can nest. |
| `describeOnly(name, () => { ... })`, `describeSkip(name, () => { ... })` | The same as `describe`, but every test of the group is `only` or `skip`. |
| `beforeEach(async () => { ... })`, `afterEach(async () => { ... })` | Run before or after each test of the current group (and its inner groups). `beforeEach` runs outer groups first; `afterEach` runs inner groups first. A throw in `beforeEach` fails the test. |
| `await render(<X />)` | Shows the element on the view screen and gives a `TestScreen`. The component runs inside the view screen, as in the app. |
| `await renderApp()` | Gives a `TestScreen` on the app as it starts: its first primary screen. Navigation in the app changes the screen that the queries search. |
| `await tick(ms?)` | Moves the test clock `ms` milliseconds on and runs each timer when it is due. |
| `await settle()` | Gives the app the host answers that wait, and runs the timers that are due in the next 60 s of test time, until nothing is left. |
| `expect(x)` | Starts an expectation (section 6). |

`TestScreen`:

| Method | What it does |
|---|---|
| `get(name)`, `row(title)`, `byText(text)`, `byRole(role, name?)` | Make a query (section 4). |
| `press(name)`, `type(name, text)`, `submit(name)`, `toggle(name)`, `select(name, value)` | Do a step on the control with this name (the same as `s.get(name).press()` and so on). |
| `showScreen(title)` | Shows the primary screen of the app with this title, as its tab or sidebar item does. |
| `text()` | `Promise<string>`: the screen as indented text, one node on each line. |
| `settle()` | The same as `settle()`. |

`Query`:

| Method | What it does |
|---|---|
| `get`, `row`, `byText`, `byRole` | A query under the node of this query: `s.row("Buy milk").get("Delete")`. |
| `press()` | Sends `press` (a button, a row, an action, a pressable). |
| `type(text)` | Sets the value of a text field, a text area or a number field and sends `change`. |
| `submit()` | Sends `submit` (the Enter key). |
| `toggle()` | Turns a toggle or a checkbox on or off (`change`). |
| `select(value)` | Selects an option of a picker or an item of tabs (`change`). The value must be one of the options. |
| `moveUp()`, `moveDown()` | Moves a row of a list with `onReorder` one place (`reorder`, as Alt+Up and Alt+Down do). |
| `text()` | `Promise<string>`: the text of the node (section 6). |

## 4. Queries

A query finds nodes **when a step or an expectation uses it**, not when the
test makes it. It searches the screen of the test: the view screen after
`render`, or the screen of the app that shows after `renderApp`.

- `get(name)`: a node whose label, title or text content is `name`.
- `row(title)`: a `Row` with this title.
- `byText(text)`: a node whose text content is `text` (`Text`, `Heading`,
  `Span`).
- `byRole(role, name?)`: a node of this control kind. The role is the
  control name of `ui-api.toml`: `"button"`, `"text-field"`, `"toggle"`,
  `"row"`, `"heading"`, and so on. With `name`, the name must also match.
- A query under a query (`s.row("A").get("Delete")`) searches only under the
  node of the first query. That first query must find exactly one node.
- The match is exact (no substring, no regular expression).
- A closed `Sheet`, `Dialog` or `Menu` hides its nodes: a query does not
  find them.

**Ambiguity.** A step, `text()`, `toHaveText`, `toContainText`,
`toBeChecked` and `toBeDisabled` need exactly one node. When more nodes
match, the test fails and the message lists each match with its place:

```
FAIL  presses a button (2 ms)
      button matches 3 nodes; make the query more exact (for example `s.row("...").get("...")`):
        button "Increment" in section "Actions"
        button "Decrement" in section "Actions"
        button "Reset" in section "Actions"
      at app/counter.test.tsx:6
```

`toExist` and `toHaveCount` count the matches; more than one is not an
error for them.

## 5. Steps and events

Each step goes through the **real event path**: the runner sends the same
`ui` event that the desktop host sends for that user action, to the handler
in the tree, with `on-event`. The core runs the handler, the effects, the
microtasks and the flush, as in the app. Thus a test sees the same
reactivity and the same ops as a user.

- A step on a disabled control fails: "cannot press button "Add task": it
  is disabled". A user cannot press it either.
- A step on a control that has no handler for the event fails, with the
  reason.
- `type`, `toggle` and `select` first set the value in the host tree, as
  the host does before the guest sees the event (SPEC §8.4).
- Write `await` before each step. A step that starts before the step before
  it ends throws "the step before it did not finish. Write `await` before
  each step."

## 6. Expectations

`expect` has a form for each kind of value. The compiler picks the form from
the type of the argument (section 10).

| `expect(...)` of | Matchers | Checked by |
|---|---|---|
| a `Query` | `toExist()`, `toHaveCount(n)`, `toHaveText(s)`, `toContainText(s)`, `toBeChecked()`, `toBeDisabled()` | the runner |
| `string` or `string \| null` | `toBe(v)`, `toEqual(v)`, `toContain(s)` | the guest |
| `number`, `int` or `number \| null` | `toBe(v)`, `toEqual(v)`, `toBeGreaterThan(v)`, `toBeGreaterThanOrEqual(v)`, `toBeLessThan(v)`, `toBeLessThanOrEqual(v)` | the guest |
| `boolean` or `boolean \| null` | `toBe(v)`, `toEqual(v)` | the guest |
| a function, no arguments | `toThrow()`, `toThrow(text)` (the error message contains `text`) | the guest |

Each form has `.not`: `expect(s.row("A")).not.toExist()`.

- **The text of a node** is its text content; else its value (a text field);
  else its title or label.
- **An expectation about a query** goes to the runner as a record. The
  runner checks it on the screen **as it is when the step ends**: at the
  next `await`, or at the end of the test. Usually that is the screen after
  the last step.
- **An expectation about a value** compares in the guest. `toEqual` is the
  same as `toBe` for these types; there is no deep equality yet.
- The first failed expectation ends the test. The runner does no more steps
  of that test.
- To read a value from the screen, use `await q.text()` or `await s.text()`,
  then `expect` the string.

## 7. Time and host answers

- **The test clock.** Each test has its own clock. `time.now()` starts at
  2026-01-01T00:00:00Z and `monotonicNow()` at 0. The clock does not move by
  itself: `setTimeout(f, 1000)` runs only after `await tick(1000)`.
- `tick(ms)` runs each timer at its time, in order, and the `onFrame`
  callbacks at 60 frames each second while the app has a frame timer.
- `settle()` waits for the host answers (`plinth:files`, `store.sql`,
  `plinth:notify`; at most 10 s of real time), gives them to the app, and
  runs the timers that are due in the next 60 s of test time.
- A test has **no network**: `fetch` gets `denied:unsupported`.
- Dialogs (`alert`, `confirm`, `prompt`) wait for an answer, and a test
  cannot answer them yet (section 12).
- When the test waits for something that does not come (no step, no timer,
  no host answer), the test fails: "the test did not finish: it waits for
  something that does not come".

## 8. Isolation

Each test runs in a **new app instance** (the runner compiles the component
one time and makes a new instance for each test):

- the module state (module-level signals and variables) starts again;
- `store.kv` is a new, empty store in memory;
- `plinth:files` and `store.sql` use a new, empty private space in a temp
  folder, which the runner removes after the test;
- the policy allows the capabilities that `plinth.toml` declares (and not
  the network);
- `Math.random` has a fixed seed, the locale is `en-US` and the time zone
  is UTC.

A test cannot see what another test did. The order of the tests does not
change the results.

## 9. The output

```
  ok  adds a task (3 ms)
FAIL  counts wrong (2 ms)
      expect(heading).toHaveText("5")
      the text is "1"
      at app/counter.test.tsx:7
      the screen:
        screen "Counter"
          section "Value"
            heading "1"
          ...
1 passed, 1 failed
```

- A failure gives the expectation or the step, the reason, the place in the
  test file (`file:line`), and the screen as text (at most 60 lines).
- An exception in the test or in the app gives "the test threw Error: ...".
  An error that the app reports (`error.report`) fails the test.
- A trap (the app stopped) gives the stop reason with its place in the app,
  from the source map (`plinth-srcmap`).
- `plinth test [dir] [--filter text]` runs only the tests whose names
  contain `text` (the group name and the test name, "group > test"). The
  summary then also counts the tests that did not run.
- A `testSkip` or `describeSkip` test does not run; it prints `skip  name`
  and the summary counts it apart: `8 passed, 0 failed, 1 skipped`. When the
  file has any `testOnly` or `describeOnly`, the other tests do not run
  either, and count in the same "skipped" total.
- The exit code is 0 when all tests pass (also when there are no tests, or
  only skips), and 1 when a test fails or the build has errors.

## 10. Compiler details

The compiler gives `plinth:test` three small things that a Plinth TS module
cannot do itself. They apply only in a test build and only to functions of
`plinth:test`.

1. **The place of a call.** A function or method of `plinth:test` whose last
   parameter is `__at?: string` gets `"app/x.test.tsx:12"` there when the
   caller leaves it out. Thus each record has the place of its step or
   expectation. This works on every engine, with no stack walk.
2. **`render(<X />)`.** `render` takes `() => JSX.Element` in Plinth TS. The
   compiler changes a JSX argument to `() => <X />`, so the element is made
   inside the view screen (in its reactive scope), not in the test. The
   typings say `render(ui: JSX.Element)` for `tsc`.
3. **Forms of `expect`.** Plinth has no unions of `string`, `number` and
   `boolean`, so `plinth:test` has `expect` (a query), `expect$string`,
   `expect$number` and `expect$boolean`. A call of `expect` picks the form
   from the type of its argument. The typings declare overloads for `tsc`.

Also, `JSX.Element` is now a valid type annotation in all Plinth TS (before,
"qualified type names are not supported").

## 11. Engines and hosts

- **Now:** `plinth test` runs the test build on the desktop runner with
  wasmtime (default) or wasmi (`PLINTH_ENGINE=wasmi`, the `wasmi` cargo
  feature of `plinth-cli`), with no window
  (`crates/plinth-cli/src/test_cmd.rs`). `Runner::new` already reads
  `PLINTH_ENGINE`, so `plinth test` needed no change: it builds the test
  runner with `--features wasmi` and runs with the env var set
  (`crates/plinth-cli/tests/plinth_test.rs`,
  `the_examples_pass_their_tests_on_wasmi`). The runner got a test clock
  (`Guest::use_test_clock`, `advance_test_clock`), a count of open host
  requests, and a cache of the last compiled component (a new instance for
  each test costs about 2 ms in a debug build).
- **Next:** the same test build on the web host (a Node runner that loads
  the core and the test build with `web/plinth-web.js` and the same
  protocol; the tree code is in `web/`). The records and the harness screen
  do not change.

## 12. Not done yet (next steps)

- Answers to dialogs (`alert`, `confirm`, `prompt`): for example
  `s.answerDialog(true)`.
- Fake network answers (`fetch` gets `denied:unsupported` now).
- Pointer and keyboard steps on Level 2 primitives (`onPointerDown`,
  `onKeyDown`), drag and drop, `onContextMenu`, sheet `onClose`, `Slider`,
  `DatePicker`.
- `navigate.back()` from the test (the host back button), and closing a
  sheet with Escape.
- Regular expressions and substrings in queries; `byLabel` for a field
  with a separate label; queries by test id.
- Deep equality for arrays and objects in `toEqual`.
- A time limit for each test other than the fixed 30 s.
- Tests on the web host (section 11), and `plinth test --watch`.
- Snapshots of the screen text, and screenshots with `plinth-shoot`.
