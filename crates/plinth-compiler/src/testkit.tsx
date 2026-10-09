// plinth:test, the test kit (docs/TESTING.md). The compiler adds this
// module to a test build (`plinth test`) only. It is Plinth TS: the test
// runner and this module talk over two channels that every core has, so
// the kit adds nothing to the core or to the WIT:
//
// - test to runner: `console.log` lines that start with MARK (the `dev`
//   interface). Each line is one record; SEP separates its fields.
// - runner to test: `change` events on the text field of the harness
//   screen (the normal UI event path).
//
// A parameter named `__at` gets the place of the call ("app/x.test.tsx:12")
// from the compiler when the caller leaves it out. `render` takes a JSX
// element; the compiler gives it to this module as a function that makes
// the element, so that the element is made in the view screen.

import { signal, Screen, TextField } from "plinth:ui";
import { console, parseNumber, int, JSON } from "plinth:core";

const MARK = "\u0001plinth:test\u0001";
const SEP = "\u001f";
const STEP = "\u001e";

function send(parts: string[]): void {
  console.log(MARK + parts.join(SEP));
}

interface TestCase {
  name: string;
  fn: () => Promise<void>;
}

const cases: TestCase[] = [];

// -- Groups: describe, beforeEach, afterEach ----------------------------------

class Group {
  name: string;
  parent: Group | null;
  only: boolean;
  skip: boolean;
  befores: (() => Promise<void>)[];
  afters: (() => Promise<void>)[];
  constructor(name: string, parent: Group | null) {
    this.name = name;
    this.parent = parent;
    this.only = false;
    this.skip = false;
    this.befores = [];
    this.afters = [];
  }
}

const rootGroup = new Group("", null);
let currentGroup: Group = rootGroup;

function joinName(group: Group, name: string): string {
  return group.name === "" ? name : `${group.name} > ${name}`;
}

function groupOnly(group: Group): boolean {
  if (group.only) {
    return true;
  }
  const p = group.parent;
  return p === null ? false : groupOnly(p);
}

function groupSkip(group: Group): boolean {
  if (group.skip) {
    return true;
  }
  const p = group.parent;
  return p === null ? false : groupSkip(p);
}

/** The chain of groups from the outermost to `group`. */
function groupChain(group: Group): Group[] {
  const p = group.parent;
  const outer: Group[] = p === null ? [] : groupChain(p);
  outer.push(group);
  return outer;
}

/** Runs `beforeEach` of `group` and its outer groups, outer first. */
async function runBefores(group: Group): Promise<void> {
  const chain = groupChain(group);
  for (let i = 0; i < chain.length; i++) {
    const fns = chain[i].befores;
    for (let j = 0; j < fns.length; j++) {
      await fns[j]();
    }
  }
}

/** Runs `afterEach` of `group` and its outer groups, inner first. */
async function runAfters(group: Group): Promise<void> {
  const chain = groupChain(group);
  for (let i = chain.length - 1; i >= 0; i--) {
    const fns = chain[i].afters;
    for (let j = 0; j < fns.length; j++) {
      await fns[j]();
    }
  }
}

/** Groups tests: the group name goes before the test name ("group > test"),
 * also in `--filter`. `describe` can nest. */
export function describe(name: string, body: () => void): void {
  const group = new Group(joinName(currentGroup, name), currentGroup);
  const outer = currentGroup;
  currentGroup = group;
  body();
  currentGroup = outer;
}

/** The same as `describe`, but only this group (and other `only` groups or
 * tests) runs when the file has any `only`. */
export function describeOnly(name: string, body: () => void): void {
  const group = new Group(joinName(currentGroup, name), currentGroup);
  group.only = true;
  const outer = currentGroup;
  currentGroup = group;
  body();
  currentGroup = outer;
}

/** The same as `describe`, but every test of this group shows as "skip". */
export function describeSkip(name: string, body: () => void): void {
  const group = new Group(joinName(currentGroup, name), currentGroup);
  group.skip = true;
  const outer = currentGroup;
  currentGroup = group;
  body();
  currentGroup = outer;
}

/** Runs before each test of the current group (and its inner groups),
 * outer groups first. A throw fails the test. */
export function beforeEach(f: () => Promise<void>): void {
  currentGroup.befores.push(f);
}

/** Runs after each test of the current group (and its inner groups),
 * inner groups first. */
export function afterEach(f: () => Promise<void>): void {
  currentGroup.afters.push(f);
}

function register(name: string, fn: () => Promise<void>, only: boolean, skip: boolean, at: string): void {
  const group = currentGroup;
  const full = joinName(group, name);
  const isOnly = only || groupOnly(group);
  const isSkip = skip || groupSkip(group);
  const wrapped = async () => {
    await runBefores(group);
    await fn();
    await runAfters(group);
  };
  cases.push({ name: full, fn: wrapped });
  send(["test", full, at, isOnly ? "1" : "", isSkip ? "1" : ""]);
}

/** Registers a test. The runner runs each test in a new app instance. */
export function test(name: string, fn: () => Promise<void>, __at?: string): void {
  register(name, fn, false, false, __at ?? "");
}

/** The same as `test`, but only this test (and other `only` tests or
 * groups) runs when the file has any `only`. */
export function testOnly(name: string, fn: () => Promise<void>, __at?: string): void {
  register(name, fn, true, false, __at ?? "");
}

/** The same as `test`, but the test shows as "skip" and does not run. */
export function testSkip(name: string, fn: () => Promise<void>, __at?: string): void {
  register(name, fn, false, true, __at ?? "");
}

// -- Steps -------------------------------------------------------------------

let resume: ((reply: string) => void) | null = null;

/** Sends a step to the runner. The promise settles when the runner has done
 * the step; its value is the reply of the runner. */
function step(parts: string[], what: string): Promise<string> {
  if (resume !== null) {
    throw new Error(`${what}: the step before it did not finish. Write \`await\` before each step.`);
  }
  send(parts);
  return new Promise<string>((resolve) => {
    resume = resolve;
  });
}

async function stepVoid(parts: string[], what: string): Promise<void> {
  await step(parts, what);
}

function command(c: string): void {
  const parts = c.split(SEP);
  if (parts[0] === "run") {
    const t = cases[int(parseNumber(parts[1]))];
    t.fn().then(
      () => send(["done"]),
      (e) => send(["error", `${e.name}: ${e.message}`]),
    );
  } else if (parts[0] === "ok") {
    const r = resume;
    resume = null;
    if (r !== null) {
      r(parts.length > 1 ? parts[1] : "");
    }
  }
}

// -- Screens that the compiler adds to a test build ---------------------------

const view = signal<(() => JSX.Element) | null>(null);

function shown(): JSX.Element {
  const f = view();
  if (f === null) {
    return null;
  }
  return f();
}

export function __View() {
  return <Screen title="plinth:test view">{shown()}</Screen>;
}

export function __Harness() {
  return (
    <Screen title="plinth:test harness">
      <TextField label="plinth:test" value="" onChange={(c) => command(c)} />
    </Screen>
  );
}

// -- Queries -------------------------------------------------------------------

/** A query: a path of steps from the screen. The runner finds the nodes. */
export class Query {
  path: string;
  desc: string;
  constructor(path: string, desc: string) {
    this.path = path;
    this.desc = desc;
  }

  child(kind: string, role: string, name: string, desc: string): Query {
    const s = kind + "\u001d" + role + "\u001d" + name;
    return new Query(this.path === "" ? s : this.path + STEP + s, this.desc === "" ? desc : `${this.desc} > ${desc}`);
  }

  /** The node with this label, title or text. */
  get(name: string): Query {
    return this.child("name", "", name, JSON.stringify(name));
  }

  /** The list row with this title. */
  row(title: string): Query {
    return this.child("name", "row", title, `row ${JSON.stringify(title)}`);
  }

  /** The node with this text content. */
  byText(text: string): Query {
    return this.child("text", "", text, `text ${JSON.stringify(text)}`);
  }

  /** The node of this role (a control name such as "button"), with this
   * name if given. */
  byRole(role: string, name?: string): Query {
    const n = name ?? "";
    return this.child(name === null ? "role" : "name", role, n, name === null ? role : `${role} ${JSON.stringify(n)}`);
  }

  act(action: string, value: string, at: string): Promise<void> {
    return stepVoid(["act", action, this.path, value, at, this.desc], `${action} ${this.desc}`);
  }

  /** Presses the button, row or other pressable control. */
  press(__at?: string): Promise<void> {
    return this.act("press", "", __at ?? "");
  }

  /** Types `text` into the text field (it replaces the value). */
  type(text: string, __at?: string): Promise<void> {
    return this.act("type", text, __at ?? "");
  }

  /** Submits the text field (the Enter key). */
  submit(__at?: string): Promise<void> {
    return this.act("submit", "", __at ?? "");
  }

  /** Turns the toggle or the checkbox on or off. */
  toggle(__at?: string): Promise<void> {
    return this.act("toggle", "", __at ?? "");
  }

  /** Selects `value` in the picker or the tabs. */
  select(value: string, __at?: string): Promise<void> {
    return this.act("select", value, __at ?? "");
  }

  /** Moves the row one place up in its list (Alt+Up). */
  moveUp(__at?: string): Promise<void> {
    return this.act("moveUp", "", __at ?? "");
  }

  /** Moves the row one place down in its list (Alt+Down). */
  moveDown(__at?: string): Promise<void> {
    return this.act("moveDown", "", __at ?? "");
  }

  /** The text of the node: its text content, value, label or title. */
  text(__at?: string): Promise<string> {
    return step(["read", this.path, __at ?? "", this.desc], `text of ${this.desc}`);
  }
}

const root = new Query("", "");

/** The screen of a test, from `render` or `renderApp`. */
export class TestScreen {
  /** The node with this label, title or text. */
  get(name: string): Query {
    return root.get(name);
  }

  /** The list row with this title. */
  row(title: string): Query {
    return root.row(title);
  }

  /** The node with this text content. */
  byText(text: string): Query {
    return root.byText(text);
  }

  /** The node of this role, with this name if given. */
  byRole(role: string, name?: string): Query {
    return root.byRole(role, name);
  }

  /** Presses the control with this label, title or text. */
  press(name: string, __at?: string): Promise<void> {
    return root.get(name).act("press", "", __at ?? "");
  }

  /** Types `text` into the text field with this label. */
  type(name: string, text: string, __at?: string): Promise<void> {
    return root.get(name).act("type", text, __at ?? "");
  }

  /** Submits the text field with this label. */
  submit(name: string, __at?: string): Promise<void> {
    return root.get(name).act("submit", "", __at ?? "");
  }

  /** Turns the toggle or the checkbox with this label on or off. */
  toggle(name: string, __at?: string): Promise<void> {
    return root.get(name).act("toggle", "", __at ?? "");
  }

  /** Selects `value` in the picker or the tabs with this label. */
  select(name: string, value: string, __at?: string): Promise<void> {
    return root.get(name).act("select", value, __at ?? "");
  }

  /** Shows the primary screen of the app with this title, as its tab or
   * its sidebar item does. */
  showScreen(title: string, __at?: string): Promise<void> {
    return stepVoid(["screen", title, __at ?? ""], `showScreen ${JSON.stringify(title)}`);
  }

  /** All the text of the screen, one node on each line. */
  text(__at?: string): Promise<string> {
    return step(["read", "", __at ?? "", "the screen"], "text of the screen");
  }

  /** Runs the timers and the host answers that wait, until none is left. */
  settle(__at?: string): Promise<void> {
    return stepVoid(["settle", __at ?? ""], "settle");
  }
}

/** Shows `ui` in the view screen of the test build. */
export async function render(ui: () => JSX.Element, __at?: string): Promise<TestScreen> {
  view.set(ui);
  await step(["render", __at ?? ""], "render");
  return new TestScreen();
}

/** Runs the app as it starts: its first screen. */
export async function renderApp(__at?: string): Promise<TestScreen> {
  await step(["renderApp", __at ?? ""], "renderApp");
  return new TestScreen();
}

/** Moves the test clock `ms` milliseconds on and runs the timers that are
 * due. */
export function tick(ms?: number, __at?: string): Promise<void> {
  return stepVoid(["tick", `${ms ?? 0}`, __at ?? ""], "tick");
}

/** Runs the timers and the host answers that wait, until none is left. */
export function settle(__at?: string): Promise<void> {
  return stepVoid(["settle", __at ?? ""], "settle");
}

// -- Expectations --------------------------------------------------------------
//
// `expect` has one form for each kind of value. The compiler picks the form
// from the type of the first argument: `expect` for a query, `expect$string`,
// `expect$number` or `expect$boolean` for a value.

function fail(message: string, at: string): void {
  send(["fail", message, at]);
  throw new Error(message);
}

export interface QueryMatchers {
  toExist: () => void;
  toHaveCount: (n: number) => void;
  toHaveText: (text: string) => void;
  toContainText: (text: string) => void;
  toBeChecked: () => void;
  toBeDisabled: () => void;
}

export interface QueryExpectation {
  toExist: () => void;
  toHaveCount: (n: number) => void;
  toHaveText: (text: string) => void;
  toContainText: (text: string) => void;
  toBeChecked: () => void;
  toBeDisabled: () => void;
  not: QueryMatchers;
}

function queryMatchers(q: Query, negate: boolean, at: string): QueryMatchers {
  const host = (matcher: string, arg: string) => {
    send(["expect", q.path, matcher, arg, negate ? "not" : "", at, q.desc]);
  };
  return {
    toExist: () => host("toExist", ""),
    toHaveCount: (n: number) => host("toHaveCount", `${n}`),
    toHaveText: (text: string) => host("toHaveText", text),
    toContainText: (text: string) => host("toContainText", text),
    toBeChecked: () => host("toBeChecked", ""),
    toBeDisabled: () => host("toBeDisabled", ""),
  };
}

/** Starts an expectation about the nodes that a query finds. The runner
 * checks it on the screen as it is when the test step ends. */
export function expect(q: Query, __at?: string): QueryExpectation {
  const at = __at ?? "";
  const m = queryMatchers(q, false, at);
  return {
    toExist: m.toExist,
    toHaveCount: m.toHaveCount,
    toHaveText: m.toHaveText,
    toContainText: m.toContainText,
    toBeChecked: m.toBeChecked,
    toBeDisabled: m.toBeDisabled,
    not: queryMatchers(q, true, at),
  };
}

export interface StringMatchers {
  toBe: (expected: string | null) => void;
  toEqual: (expected: string | null) => void;
  toContain: (part: string) => void;
}

export interface StringExpectation {
  toBe: (expected: string | null) => void;
  toEqual: (expected: string | null) => void;
  toContain: (part: string) => void;
  not: StringMatchers;
}

function showString(v: string | null): string {
  return v === null ? "null" : JSON.stringify(v);
}

function stringMatchers(v: string | null, negate: boolean, at: string): StringMatchers {
  const not = negate ? "not " : "";
  const check = (ok: boolean, what: string) => {
    if (ok === negate) {
      fail(`expected ${showString(v)} ${not}${what}`, at);
    }
  };
  return {
    toBe: (e: string | null) => check(v === e, `to be ${showString(e)}`),
    toEqual: (e: string | null) => check(v === e, `to equal ${showString(e)}`),
    toContain: (part: string) => check(v !== null && v.includes(part), `to contain ${showString(part)}`),
  };
}

export function expect$string(v: string | null, __at?: string): StringExpectation {
  const at = __at ?? "";
  const m = stringMatchers(v, false, at);
  return { toBe: m.toBe, toEqual: m.toEqual, toContain: m.toContain, not: stringMatchers(v, true, at) };
}

export interface NumberMatchers {
  toBe: (expected: number | null) => void;
  toEqual: (expected: number | null) => void;
}

export interface NumberExpectation {
  toBe: (expected: number | null) => void;
  toEqual: (expected: number | null) => void;
  not: NumberMatchers;
}

function showNumber(v: number | null): string {
  return v === null ? "null" : `${v}`;
}

function numberMatchers(v: number | null, negate: boolean, at: string): NumberMatchers {
  const not = negate ? "not " : "";
  const check = (ok: boolean, what: string) => {
    if (ok === negate) {
      fail(`expected ${showNumber(v)} ${not}${what}`, at);
    }
  };
  return {
    toBe: (e: number | null) => check(v === e, `to be ${showNumber(e)}`),
    toEqual: (e: number | null) => check(v === e, `to equal ${showNumber(e)}`),
  };
}

export function expect$number(v: number | null, __at?: string): NumberExpectation {
  const at = __at ?? "";
  const m = numberMatchers(v, false, at);
  return { toBe: m.toBe, toEqual: m.toEqual, not: numberMatchers(v, true, at) };
}

export interface BooleanMatchers {
  toBe: (expected: boolean | null) => void;
  toEqual: (expected: boolean | null) => void;
}

export interface BooleanExpectation {
  toBe: (expected: boolean | null) => void;
  toEqual: (expected: boolean | null) => void;
  not: BooleanMatchers;
}

function showBoolean(v: boolean | null): string {
  return v === null ? "null" : v ? "true" : "false";
}

function booleanMatchers(v: boolean | null, negate: boolean, at: string): BooleanMatchers {
  const not = negate ? "not " : "";
  const check = (ok: boolean, what: string) => {
    if (ok === negate) {
      fail(`expected ${showBoolean(v)} ${not}${what}`, at);
    }
  };
  return {
    toBe: (e: boolean | null) => check(v === e, `to be ${showBoolean(e)}`),
    toEqual: (e: boolean | null) => check(v === e, `to equal ${showBoolean(e)}`),
  };
}

export function expect$boolean(v: boolean | null, __at?: string): BooleanExpectation {
  const at = __at ?? "";
  const m = booleanMatchers(v, false, at);
  return { toBe: m.toBe, toEqual: m.toEqual, not: booleanMatchers(v, true, at) };
}
