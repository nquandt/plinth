// plinth:test: the test kit (docs/TESTING.md). Only test files
// (`*.test.ts`, `*.test.tsx`) can import it, and only `plinth test` builds
// them. A test build never goes into a package.
//
//   import { test, render, expect } from "plinth:test";
//   import Tasks from "./tasks";
//
//   test("adds a task", async () => {
//     const s = await render(<Tasks />);
//     await s.type("Task", "Buy milk");
//     await s.press("Add task");
//     expect(s.row("Buy milk")).toExist();
//   });
//
// Each step that changes the screen (`render`, `press`, `type`, `tick`, ...)
// returns a `Promise`: write `await` before it. An expectation about a query
// checks the screen as it is when the step ends (at the next `await`, or at
// the end of the test).

/** Registers a test. Each test runs in a new app instance: the module
 * state, the `kv` store and the private files are new. */
export declare function test(name: string, fn: () => Promise<void>): void;

/** Shows `ui` (for example `<Tasks />`) on the view screen of the test
 * build, and gives the screen to query. */
export declare function render(ui: JSX.Element): Promise<TestScreen>;

/** Gives the app as it starts: its first screen. Navigation in the app
 * changes the screen that the queries search. */
export declare function renderApp(): Promise<TestScreen>;

/** Moves the test clock `ms` milliseconds on (0 if not given) and runs
 * each timer when it is due. The clock does not move by itself. */
export declare function tick(ms?: number): Promise<void>;

/** Gives the app the host answers that wait (files, network: a test has
 * no network) and runs the timers that are due in the next 60 s of test
 * time, until none is left. */
export declare function settle(): Promise<void>;

/** A query: it finds nodes on the screen when a step or an expectation
 * uses it. A step needs exactly one node; when more nodes match, the test
 * fails and lists them. */
export declare class Query {
  /** The node with this label, title or text, under this node. */
  get(name: string): Query;
  /** The list row with this title, under this node. */
  row(title: string): Query;
  /** The node with this text content, under this node. */
  byText(text: string): Query;
  /** The node of this role (a control name, such as "button",
   * "text-field" or "toggle"), with this name if given. */
  byRole(role: string, name?: string): Query;
  /** Presses the button, row or other pressable control. */
  press(): Promise<void>;
  /** Types `text` into the text field (it replaces the value). */
  type(text: string): Promise<void>;
  /** Submits the text field (the Enter key). */
  submit(): Promise<void>;
  /** Turns the toggle or the checkbox on or off. */
  toggle(): Promise<void>;
  /** Selects `value` in the picker or the tabs. */
  select(value: string): Promise<void>;
  /** Moves the row one place up in its list (Alt+Up). */
  moveUp(): Promise<void>;
  /** Moves the row one place down in its list (Alt+Down). */
  moveDown(): Promise<void>;
  /** The text of the node: its text content, its value, or its title or
   * label. */
  text(): Promise<string>;
}

/** The screen of a test, from `render` or `renderApp`. */
export declare class TestScreen {
  /** The node with this label, title or text. */
  get(name: string): Query;
  /** The list row with this title. */
  row(title: string): Query;
  /** The node with this text content. */
  byText(text: string): Query;
  /** The node of this role, with this name if given. */
  byRole(role: string, name?: string): Query;
  /** Presses the control with this label, title or text. */
  press(name: string): Promise<void>;
  /** Types `text` into the text field with this label. */
  type(name: string, text: string): Promise<void>;
  /** Submits the text field with this label. */
  submit(name: string): Promise<void>;
  /** Turns the toggle or the checkbox with this label on or off. */
  toggle(name: string): Promise<void>;
  /** Selects `value` in the picker or the tabs with this label. */
  select(name: string, value: string): Promise<void>;
  /** Shows the primary screen of the app with this title, as its tab or
   * its sidebar item does. Then the queries search that screen. */
  showScreen(title: string): Promise<void>;
  /** All the text of the screen: one node on each line, indented. */
  text(): Promise<string>;
  /** The same as the function `settle()`. */
  settle(): Promise<void>;
}

/** Expectations about the nodes that a query finds. */
export interface QueryMatchers {
  /** The query finds one node or more. */
  toExist(): void;
  /** The query finds exactly `n` nodes. */
  toHaveCount(n: number): void;
  /** The text of the node is `text`. */
  toHaveText(text: string): void;
  /** The text of the node contains `text`. */
  toContainText(text: string): void;
  /** The toggle or the checkbox is on. */
  toBeChecked(): void;
  /** The control is disabled. */
  toBeDisabled(): void;
}

export interface QueryExpectation extends QueryMatchers {
  /** The opposite of each expectation. */
  not: QueryMatchers;
}

export interface StringMatchers {
  toBe(expected: string | null): void;
  toEqual(expected: string | null): void;
  toContain(part: string): void;
}

export interface StringExpectation extends StringMatchers {
  not: StringMatchers;
}

export interface NumberMatchers {
  toBe(expected: number | null): void;
  toEqual(expected: number | null): void;
}

export interface NumberExpectation extends NumberMatchers {
  not: NumberMatchers;
}

export interface BooleanMatchers {
  toBe(expected: boolean | null): void;
  toEqual(expected: boolean | null): void;
}

export interface BooleanExpectation extends BooleanMatchers {
  not: BooleanMatchers;
}

/** Starts an expectation about a query, a string, a number or a boolean.
 * A failed expectation ends the test. */
export declare function expect(query: Query): QueryExpectation;
export declare function expect(value: string | null): StringExpectation;
export declare function expect(value: number | null): NumberExpectation;
export declare function expect(value: boolean | null): BooleanExpectation;
