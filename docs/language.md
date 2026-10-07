# Plinth TS language reference

Plinth TS is a strict, statically typed subset of TypeScript with JSX.
Every valid Plinth TS program is valid `.tsx`: `tsc`, editors, and
formatters work on it with no changes. The Plinth compiler is the final
authority. It rejects programs that `tsc` accepts but that use a feature
outside the subset, with a stable error code and a suggested fix.

## Why a subset

Plinth compiles your app straight to a small WebAssembly module with no
garbage collector built into Wasm (SPEC.md §5.4), and it must prove,
mechanically, which host capabilities your app can reach (SPEC.md §11).
Both of these goals need a closed, checkable language. The subset is
large enough to write real apps; see the examples in `examples/`.

## Types

| Plinth TS | Notes |
|---|---|
| `number` | A 64-bit float (IEEE 754 `f64`). The globals `Infinity` and `NaN` exist. |
| `int` | A branded `number` from `plinth:core`. Literals and arithmetic on `int` stay `int` and wrap at 32 bits. `int` converts to `number` implicitly; convert back with `int(x)` (it truncates). |
| `boolean` | — |
| `string` | Immutable. `length`, indexing, and `slice` count UTF-16 code units, for TypeScript compatibility; see "Deviations" below. |
| `T[]` / `Array<T>` | A growable array, monomorphized per `T`. See `std/lib.d.ts` for the supported methods (`push`, `pop`, `map`, `filter`, `find`, `findIndex`, `some`, `every`, `forEach`, `includes`, `indexOf`, `slice`, `reverse`, `join`, `concat`, `reduce` (needs an initial value; there is no no-initial-value overload), `sort` (in place, stable; without a comparator, `string`/`number`/`boolean` elements only, compared as strings exactly like JS, with a lint for `number`), `splice` (JS rules: a negative start counts from the end; it returns the removed elements), `fill(value, start?, end?)` (in place), `flat(depth?)` (the depth is a number literal or `Infinity`; a depth larger than the nesting flattens all levels; a computed depth is the error `PL2000`), `for…of`). |
| `Map<K, V>`, `Set<T>` | `K`/element type must be `string`, `int`, `number`, `boolean`, or an enum. Insertion order; lookup, insert, and delete are linear scans. `get` returns `null`, not `undefined`, for a missing key. `forEach`, `keys()`, `values()` and `entries()` work as plain methods too, not just as a `for…of` target (`[...m.keys()]`, `m.values().reduce(...)`, `m.entries().map(([k, v]) => ...)`). `entries()` returns a new `[K, V][]`; a change to it does not change the map. `for (const e of m)` gives `e` as a `[K, V]` pair. `std/lib.d.ts` declares `Map` and `Set` iterable, so `tsc` also accepts `for…of` over them. |
| `[A, B]` (tuples) | A tuple. It is a struct at run time. Read an element with a number literal index (`t[0]`) or with destructuring (`const [a, b] = t`). A computed index is an error (`PL2011`). `t.length` works. **Optional elements** (`[number, string?]`) come after the required ones; an optional element has the type `T \| null`, and a literal can leave it out (it is then `null`). **A rest element** (`[string, ...number[]]`) must be last and have an array type; a literal can have any number of elements after the fixed ones, also spreads (`["a", ...xs]`). `[...T[]]` is the same as `T[]`. Not supported: a rest element that is not last (`PL2012`), a rest element that is not an array, such as `...[A, B]` (`PL2012`), and a rest pattern on a tuple without a rest element (`PL2000`). **Rest patterns** work on arrays (`const [a, , ...r] = xs`: `r` is a new array) and on tuples with a rest element (`const [s, n, ...more] = t`, after the fixed elements). `JSON.stringify` writes a tuple as a JSON array (a left-out optional element is `null`); `JSON.parse` does not accept a tuple type. |
| `T \| null` | Supported for every type except signals. `undefined` exists only as "absent" for optional properties and parameters; it is otherwise the same value as `null`. |
| interfaces, object types | A GC-allocated struct with a fixed layout. |
| classes | Single inheritance: fields, one constructor, methods (also `async` methods), `new`, `extends`, `super(...)` as the first statement, `super.m()`, overriding, `instanceof` with narrowing. A base class must be declared before its subclasses. A method reference without a call (`arr.map(obj.method)`) is the error `PL2021`; write `() => obj.method()` instead. |
| union types | Tagged. Narrow with `typeof`, a discriminant field, `instanceof`, or `=== null`. |
| enums, string literal unions | Compile to `i32`/interned ids. |
| generics | Monomorphized: functions, interfaces, and type aliases. No constraints, no defaults, no conditional or mapped types. |
| `Promise<T>` | The result of an `async` function, of a host call without a `done` callback, or of `new Promise<T>(...)`. Read it with `await` or `then`. See "Async functions and `await`" and "The `Promise` API". |

**Status (current).** Implemented: `int`; `T | null` everywhere except
signals; discriminated unions with narrowing; generic functions,
interfaces, and type aliases; `Map`/`Set`; classes with single
inheritance and `instanceof`; `JSON.stringify`/`JSON.parse<T>`;
`try`/`catch`/`finally`/`throw` (see "Errors and exceptions");
`async`/`await` (see "Async functions and `await`"); the `Promise` API
with `all`, `race`, `any` and `allSettled`. **Not yet:** static
members, getters/setters, generic classes.

## Supported syntax

- `let`, `const`, `function`, arrow functions, closures, default and
  optional parameters.
- `if`/`else`, `switch`, `for`, `for…of` (arrays, `Map`, `Set`), `while`,
  `break`, `continue`, `return`.
- Arithmetic, comparison (`===`/`!==` only, no `==`/`!=`), logical
  operators, `??`, `?.`, the ternary operator, template literals.
- Object literals, array literals, destructuring, spread into arrays and
  objects of a known shape.
- `interface`, `type` aliases, string literal unions, `enum`.
- JSX with Plinth control tags only (see [ui.md](ui.md)). A fragment
  (`<>...</>`) is allowed directly in a JSX child position — it
  creates no node of its own; the parser splices its children straight
  into the parent's child list at compile time
  (`<Section><>{a}{b}</></Section>` is valid, and is exactly the same
  tree as `<Section>{a}{b}</Section>`). This flattening is purely
  syntactic, so a fragment **inside a dynamic expression** (one branch
  of a ternary, a `.map()` callback, …) is not supported — that child
  slot can only ever hold a single element at run time today — and
  neither is a fragment as a component's return value or any other
  standalone expression, because lowering has no caller-side parent to
  splice its children into there. For a conditional block with more
  than one sibling, wrap the elements in a `<Section>` or `<Group>` and
  put the condition inside it instead of around it.
- `{arr.map(x => <Row .../>)}` as a JSX child (optionally after a
  `.filter(...)`, since `filter` already returns a plain array): the
  checker desugars it to the same keyed `List` the reconciler uses for
  a literal `<List items key row>`, keyed by position (array index),
  with no node of its own for the `.map` itself. Only this exact shape
  is recognized — a `.map` call whose argument is an inline
  arrow/function expression that returns an element. A named function
  reference (`arr.map(row)`) or any other array-producing expression
  as a JSX child is still rejected (`PL4004`); use a literal `<List
  items={...} key={...} row={...} />` for those, since a hand-written
  `key` lets the reconciler track identity across a reorder instead of
  by position.
- `import`/`export` within the closed world (see below).
- Classes, single inheritance, `instanceof`, general union narrowing,
  generic functions and interfaces (all implemented; see "Status"
  above).
- Narrowing on member expressions, not just plain variables: `if (r.subtitle
  !== null) { use(r.subtitle); }`, `r.subtitle !== null ? r.subtitle : "x"`,
  `r.subtitle === null ? "x" : r.subtitle`, and the same for `if (... ===
  null) return;`/`throw` at the end of a block narrowing the rest of it.
  `r.subtitle ?? "x"` and `r?.subtitle ?? "x"` already typed correctly
  without this (`??` and `?.` build their own null check, independent of
  the narrowing map). The rule, deliberately conservative: a path `a.b` or
  `a.b.c` is narrowed only when `a` is a `const` or a parameter (never
  reassigned) and every step is a direct struct field read — not a method
  call, not a computed/array index. The narrowing is dropped (not carried
  past) any assignment to any field, or any function/method call, anywhere
  in between the test and the use, since either could change the field
  through an alias that the checker cannot rule out. This is strictly more
  conservative than necessary (a call that provably cannot reach the path
  still drops it), by design: it costs an extra local variable in the rare
  case that trips it, in exchange for never needing an alias analysis.
- `expr as T`, but only for a checked, safe narrowing: `string` (or a
  wider string literal union) to a narrower string literal union, or a
  discriminated union to one of its members. Each accepted cast carries
  its own run-time check and `throw`s (traps) if the value turns out
  not to match — Plinth has no `any` escape hatch to fall back on, so
  an `as` that TypeScript treats as a compile-time-only annotation must
  still be safe at run time here. `number as int` is deliberately not
  one of these: there is nothing to check that would make it safe, so
  it is rejected; use `int(x)` (it truncates). Any other cast, between
  types the checker cannot relate this way, is still rejected with
  `PL2006`.


## Async functions and `await`

An `async` function returns a `Promise<T>`. Inside it, `await p` stops
the function until the promise `p` settles, and gives its value. Other
code (event handlers, timers) runs while the function waits.

```ts
import { confirm, prompt, alert } from "plinth:dialog";
import { fetch } from "plinth:net";

async function greet(): Promise<string> {
  const name = await prompt("Your name?");      // string | null
  if (name === null) {
    return "nobody";
  }
  if (await confirm(`Greet ${name}?`)) {
    await alert(`Hello, ${name}!`);
  }
  return name;
}

<Button label="Greet" onPress={async () => status.set(await greet())} />
```

Rules:

- **Host calls.** A host call without its `done` callback returns a
  promise: `alert(m)` (`Promise<void>`), `confirm(m)`
  (`Promise<boolean>`), `prompt(m)` (`Promise<string | null>`),
  `fetch(url, options)` (`Promise<Response>`), and `plinth:hub`'s
  `search`, `install`, `checkUpdates` and `update`
  (`Promise<string | null>`). The forms with `done` do not change. These
  promises never reject: a denied call gives its normal denied value.
- **`async` functions.** `async function`, `async` arrow functions and
  `async` function expressions. Write the return type as `Promise<T>`, or
  let the compiler infer it. An `async` arrow function where the context
  expects a function that returns `void` (an event handler, a `forEach`
  callback) returns nothing: its promise is detached.
- **`async` methods.** A class method can be `async`. Its body can use
  `this` after an `await` and can `await` other methods, also
  `super.m()`. An override of an `async` method must also return a
  `Promise` of the same type. Generator methods are the error `PL2009`.
- **Where `await` can be.** In any expression of an `async` function,
  also inside `if`, `while`, `do…while`, `for`, `for…of`, `switch`,
  blocks and `try`/`catch`/`finally`. The compiler keeps the order of
  evaluation: the parts of an expression before an `await` are evaluated
  before it waits, and the right side of `&&`, `||` or `?:` waits only if
  it runs. A `finally` block runs after an `await` in the `try` or
  `catch` block, also when they `return`, `throw`, `break` or
  `continue`. `await` in a `case` value is the error `PL2009` (compute
  the value before the `switch`). `await` outside an `async` function is
  `PL2009` (at the top level of a module) or a syntax error (`PL1000`).
- **Errors.** A `throw` in an `async` function rejects its promise.
  `await` of a rejected promise throws its error, so `try`/`catch` works
  around `await`. A rejected promise that nothing awaits is reported to
  the host as `Uncaught (in promise) Error: …` at the end of the event;
  the app keeps running.
- **Order.** As in JavaScript, the code after an `await` always runs
  later, as a microtask, even when the promise has already settled. The
  runtime runs the microtasks after each event and before it renders, so
  a signal that the code after an `await` sets shows in the same frame.
- **Memory.** The variables that the code after an `await` uses move to
  the heap. A waiting function stays alive while the host request that
  it waits on is open.

**How it works.** The compiler rewrites an `async` function into
closures (a continuation-passing transform, `check/asyncfn.rs`): the code
after each `await` becomes a continuation that the promise calls when it
settles. A loop with an `await` becomes a loop closure. An iteration that
does not wait stays in one Wasm loop; a continuation starts the next
iteration as a new microtask, so a long loop does not grow the stack.
`Promise<T>`, the microtask queue and the unhandled-rejection list are
generated app code. The runtime only calls the queue's drain function
after each event (core 1.10: `set_drain`, `report`), so an app without
`async` code pays nothing.

A `switch` with an `await` becomes `if` statements that select the case
and run the case bodies with fall-through. A `do…while` with an `await`
becomes a `while` loop with a flag. A `finally` block with an `await`
becomes a closure; the end of the `try` and `catch` blocks, an
exception, and a `return`, `break` or `continue` that leaves them call
it, and then it completes what was pending.

**Cost.** Each `async` function allocates a promise and a few closures
per call. All promise types share one `then`, `reject` and `settle`
function; the first use of each `Promise<T>` type adds one small
`resolve` function. `examples/dialogs` has 5673 B of app code (6075 B
with one set of helpers for each promise type).

## The `Promise` API

```ts
// Run three requests at the same time; the values keep the order.
const pages = await Promise.all(urls.map((u) => fetch(u, null)));
// Promises of different types give a tuple.
const [n, name] = await Promise.all([count(), prompt("Name?")]);

// A promise from a callback API.
function delay(ms: number): Promise<void> {
  return new Promise<void>((resolve) => { setTimeout(resolve, ms); });
}

load()
  .then((items) => items.length)
  .catch((e) => -1)
  .finally(() => busy.set(false));
```

- **`p.then(onFulfilled, onRejected?)`** returns a new promise. Each
  callback can return a value or a promise (the new promise then waits
  for it). Without `onRejected`, a rejection goes to the new promise.
  `onRejected` gets an `Error` and must return the same type as
  `onFulfilled`.
- **`p.catch(onRejected)`** returns a `Promise<T>` of the same `T`, so
  `onRejected` must return a `T` (or a `Promise<T>`; anything for
  `Promise<void>`).
- **`p.finally(f)`** runs `f` when `p` settles, then gives the same
  value or error. If `f` returns a promise, the result waits for it; a
  rejection of that promise replaces the result.
- An exception in a callback rejects the new promise.
- **`new Promise<T>((resolve, reject) => { ... })`.** The executor runs
  at once. For `Promise<void>`, `resolve` takes no argument. `reject`
  takes an `Error`. The first call of `resolve` or `reject` settles the
  promise; later calls do nothing. An exception in the executor rejects
  the promise. Write `<T>`, or use `new Promise` where the type is known
  (a `return` of a function with a `Promise<T>` return type, or a
  variable with a type); otherwise it is the error `PL3007`.
- **`Promise.all(ps)`.** For an array of `Promise<T>`, a
  `Promise<T[]>` with the values in the order of the array; for
  `Promise<void>[]`, a `Promise<void>`. For an array literal of promises
  of different types, a promise of a tuple (`[A, B]`); a `Promise<void>`
  cannot be in such a tuple. The result rejects with the first
  rejection. An empty array gives an empty array at once.
- **`Promise.resolve(v)`** gives a fulfilled promise (`Promise.resolve()`
  gives a `Promise<void>`); a promise argument is returned as it is.
  **`Promise.reject(e)`** gives a rejected promise; `e` is an `Error` or
  a string (`new Error(e)`). Write `Promise.reject<T>(e)` for a type
  other than `Promise<void>`.
- **`Promise.race(ps)`** settles as the first promise in `ps` that
  settles: the same value or the same error. An empty array gives a
  promise that never settles.
- **`Promise.any(ps)`** resolves with the first value. When all
  promises reject, it rejects with an `Error` whose `name` is
  `"AggregateError"` and whose `message` is "All promises were
  rejected" (Plinth has no `AggregateError` class, so the single errors
  are not kept). An empty array rejects at once.
- **`Promise.allSettled(ps)`** waits until all promises settle and
  never rejects. It gives an array of `PromiseSettledResult<T>`, in the
  order of `ps`. This is a discriminated union, as in TypeScript:
  `{ status: "fulfilled"; value: T }` (`PromiseFulfilledResult<T>`) or
  `{ status: "rejected"; reason: Error }` (`PromiseRejectedResult`).
  Narrow it with `r.status === "fulfilled"`. For `Promise<void>`, the
  fulfilled result has no `value` field.
  ```ts
  const rs = await Promise.allSettled(urls.map((u) => fetch(u, null)));
  for (const r of rs) {
    if (r.status === "fulfilled") { show(r.value.body); } else { log(r.reason.message); }
  }
  ```
- `race`, `any` and `allSettled` take an array of promises of one type.
  An array literal of promises of different types is the error `PL2012`
  (only `Promise.all` makes a tuple).
- Not supported: other `Promise` functions such as
  `Promise.withResolvers` (`PL3004`), and a `then` callback that takes a
  rejection reason of a type other than `Error`.

## Errors and exceptions

Plinth has `throw`, `try`, `catch` and `finally` (SPEC.md §5.6).

```ts
class NotFound extends Error {
  id: string;
  constructor(id: string) {
    super(`no item ${id}`);
    this.name = "NotFound";
    this.id = id;
  }
}

try {
  load(id);
} catch (e) {
  if (e instanceof NotFound) {
    status.set(`missing: ${e.id}`);
  } else {
    status.set(e.message);
  }
} finally {
  busy.set(false);
}
```

Rules:

- `throw` takes an `Error` or an instance of a subclass of `Error`.
  `Error` is a built-in class with the fields `name` and `message`
  (`std/lib.d.ts`). `throw "text"` throws `new Error("text")`. Other
  values are the error `PL3001`.
- A `catch` variable has the type `Error`. Narrow it with `instanceof`.
  You can write `catch (e)`, `catch (e: unknown)` (the `tsc` form) or
  `catch` with no variable. Another type annotation, or a pattern, is
  the error `PL2010`. You cannot assign to the variable.
- `finally` runs when the `try` or `catch` block completes, returns,
  throws, or leaves a loop with `break` or `continue`. An exception in a
  `finally` block replaces the exception that was pending.
- An exception goes through function calls, closure calls, method calls
  and array callbacks (`map`, `forEach`, …).
- **Runtime traps are not exceptions.** A read outside an array, a
  failed `as` cast, a `null` used as a number, and out-of-memory stop
  the app. No `catch` gets them.
- **Uncaught exceptions.** An exception that no `catch` gets in an event
  handler, a timer, a host-call callback, an effect, a `computed` or a
  render slot goes to the host through `error.report` (core 1.10), for
  example `Uncaught NotFound: no item 7`. The app keeps running. The
  signal changes before the `throw` stay. A `computed` or a slot that
  throws gives the zero value of its type. An uncaught exception in the
  start-up code (the top level of a module, or a screen component) is
  also reported, but then the app stops, because it has no complete UI.

**Why generated code, not the Wasm exception-handling proposal.** The
compiler does not use the Wasm `try`/`catch` instructions. It generates
the exception path:

1. Two globals hold the pending exception: a flag and the `Error`.
2. `throw` sets them and branches to the innermost `catch` block of the
   function, or returns from the function.
3. After each call of a function that can throw, the caller tests the
   flag and continues the exception path. A may-throw analysis finds
   these functions. A closure call is tested only when some closure in
   the program can throw.
4. A thunk (where the runtime calls app code) is the outer boundary. It
   reports a pending exception and clears it.

We chose this because:

- **It runs on every target now.** SPEC.md §9.2 requires the iOS
  ahead-of-time path (`wasm2c`, WAMR AOT) in addition to wasmtime and
  browsers. These runners do not support the same version of the
  exception-handling proposal (the older `try`/`catch` form or the newer
  `try_table`/`exnref` form), and some AOT tools support neither. The
  generated code uses only Wasm 1.0 instructions.
- **It costs nothing when nothing throws.** An app with no `throw` gets
  no tests and no globals, and it does not import the core 1.10 function
  `uncaught`. An app with a `throw` pays one `global.get` and one `br_if`
  after each call that can throw.
- **It fits the runtime.** The garbage collector runs only between
  events (SPEC.md §5.4), so an exception never needs to unwind a shadow
  stack. The thunk boundary gives one place to report an uncaught
  exception without a trap.
- **It is the base for `async`.** A continuation of an `async` function
  must catch every exception and reject its promise. The same mechanism
  does that.

The cost: a call that can throw is a little larger and slower than with
native exceptions. When every target runner supports one version of the
proposal, a later compiler can change to it without a change to the
language.

## Rejected features, and why

The compiler rejects these, each with a clear error and a suggested fix:

- `any`, `unknown` casts that bypass the checker, `as` casts between
  unrelated types, `!` non-null assertions the checker cannot prove.
  *Why:* the checker must know every value's exact type to generate
  code and to check capabilities.
- `eval`, `new Function`, `with`, `arguments`, `this` outside class
  methods. *Why:* these need a dynamic, interpreted environment that a
  compiled, no-GC-heap runtime cannot support.
- `==`, `!=`, `delete`, `in` on objects, `for…in`. *Why:* object layouts
  are fixed at compile time; there is no dynamic property list to
  iterate or delete from.
- Computed property access on a plain object (`obj[key]` where `obj` is
  not a `Map` or an array). *Why:* same reason — use a `Map` instead.
- Prototype access or changes, `Object.defineProperty`, `Proxy`,
  `Reflect`, `Symbol` (other than the built-in iterator protocol).
  *Why:* there is no prototype chain at run time.
- Getters/setters, decorators, namespaces, `declare` blocks in app code.
  *Why:* not yet implemented, or incompatible with the closed world.
- Conditional types, mapped types, `infer`, template literal types,
  index access types. *Why:* the compiler does not evaluate
  type-level code; it needs concrete types to generate Wasm.
- Ambient globals of any kind (`document`, `window`, `process`,
  `fetch`, …). *Why:* the closed world (below) is how the compiler
  proves what your app can reach.

**Structural typing.** TypeScript is structural, and Plinth keeps
structural *assignment* for object types and interfaces: if a value's
shape matches an interface, it can be assigned to it. When the compiler
cannot prove the same layout at compile time, it builds an **itable** (a
per-struct-per-interface table of field offsets and method indices) so
the access still works, at the cost of one extra indirection.

## The closed world

A Plinth app can import only two kinds of thing:

1. Relative paths inside the app (`./home`, `../lib/x`).
2. `plinth:<module>` — the framework's own modules.

Any other import (`"lodash"`, `"react"`, a bare npm package name) is a
compile error. The compiler never reads `node_modules`. This is enforced
at three levels:

1. **The editor.** The generated `tsconfig.json` sets `"noLib": true`
   and `"types": []`, so `tsc` itself rejects ambient globals and bare
   imports before you ever run `plinth check`.
2. **The compiler.** The import resolver only accepts the two forms
   above.
3. **The artifact.** `plinth validate` (and every host) rejects a
   compiled package whose Wasm module imports anything outside the
   declared `plinth:*` interfaces. Opening a package grants it nothing.

## Reactivity

```ts
import { signal, computed, effect } from "plinth:ui";

const count = signal(0);              // Signal<int>: count() reads; .set(v) / .update(f) write
const double = computed(() => count() * 2);
effect(() => console.log(`${count()}`));
```

Rules:

- A **signal** holds a value. Call it to read; `.set(value)` or
  `.update(f)` to write.
- A **computed** is a derived, read-only signal.
- An **effect** reruns whenever a signal it read last time changes.
- Reads inside a *tracking scope* — a JSX slot, a `computed`, or an
  `effect` — are tracked. Writes mark their dependents dirty. The
  scheduler flushes all pending updates once, after the current event
  handler returns, and writes the minimal set of UI ops.
- **A component function runs once per instance.** It is not a
  template that re-renders. If you read a signal directly in a
  component's body, outside JSX, `computed`, or an event handler, that
  read happens once and will not update:

  ```tsx
  function Home() {
    const count = signal(0);
    const n = count();              // PL2020 warning
    return <Text>{`n is ${n}`}</Text>;
  }
  ```
  ```
  app/main.tsx(5,13): warning PL2020: this signal read is not reactive: it runs once and will not update
    help: move the read into the JSX or a `computed`
  ```
  Fix it by reading the signal inside the JSX (`{count()}`) or wrapping
  the derived value in `computed(() => ...)`.

## Deviations from JavaScript/TypeScript

- **Number semantics.** `int` arithmetic wraps at 32 bits; `number`
  behaves as IEEE 754 `f64`, same as JS.
- **Strings.** Stored as UTF-8 at run time, but `length`, indexing, and
  `slice` count UTF-16 code units, matching TS/JS behavior, with an
  ASCII fast path.
- **`null` and `undefined`** are one value at run time.
- **Object identity.** `===` on objects compares references, as in JS.
  There is no `==`.
- **Property order.** Object literal fields keep their declared order.
  Iterating over an object's keys is not supported; use a `Map`.
- **`await` and microtasks.** The code after an `await` runs as a
  microtask, as in JavaScript. A continuation of a loop iteration that
  did not really wait also starts the next iteration as a microtask, not
  at once. A rejection reason is always an `Error`.
- **Errors.** `throw` takes only an `Error` (or a subclass); a thrown
  string becomes an `Error`. A `catch` variable is an `Error`, not
  `unknown`, and you cannot assign to it. Runtime errors (null
  dereference, out-of-bounds index, a failed `as` cast, out-of-memory)
  are traps, not exceptions: no `catch` gets them, and the host shows a
  "this app stopped" screen. See "Errors and exceptions".
- **`for (let i = ...)`** uses one binding for the whole loop — closures
  made inside the body see its final value, matching old-style `var`
  behavior rather than per-iteration `let`. `for…of` loop variables, and
  variables declared inside the loop body, do get a fresh binding per
  iteration, as in JS.
- **`toUpperCase`/`toLowerCase`** map ASCII, Latin-1, Latin Extended-A,
  Greek, and Cyrillic. Other scripts pass through unchanged, and
  multi-character mappings (`ß` → `SS`) are not performed.
- **`parseNumber`** (the `plinth:core` equivalent of JS `Number(s)`) is
  exact for up to 15 significant digits and a decimal exponent from -22
  to 22; other inputs can differ by one unit in the last place. Hex and
  binary literals are not parsed.
- **`toFixed`** is exact, except for integers above 2^64, which fall
  back to `toString`'s digits.

## The standard library (`plinth:*` modules)

Each module has a hand-written `.d.ts` file in `std/`, which your editor
reads. The compiler keeps the same signatures built in, and a test
(`std_typings_match`) keeps the two in sync, so `std/*.d.ts` is always
accurate.

| Module | Contents | Capability |
|---|---|---|
| `plinth:core` | `int`, `Math` (with `Math.random`, core 1.12: the host seeds it with fresh entropy, a test with a fixed seed), `seedRandom(seed)`, `JSON.stringify`/`JSON.parse<T>`, `parseNumber`, `toString`, `console.log` (dev only) | none |
| `plinth:ui` | `signal`, `computed`, `effect`, `app()`, `navigate`, and every UI control (see [ui.md](ui.md)) | none |
| `plinth:time` | `now()`, `monotonicNow()`, `setTimeout`/`setInterval`/`clearTimeout`/`clearInterval` | none |
| `plinth:store` | `kv.get`/`kv.set`/`kv.remove`/`kv.keys`, `kv.lastError()` | `store.kv` |
| `plinth:clipboard` | `writeText`, `readText`, `lastError()` | `clipboard.write` / `clipboard.read` |
| `plinth:dialog` | `alert`, `confirm`, `prompt` (host-owned modal dialogs) | none |
| `plinth:net` | `fetch` (HTTP, text bodies), with a `done` callback or as a `Promise` | `net:<host>` or `net:*`, and `net.local` for a private address |
| `plinth:files` | `read`, `write`, `list`, `stat`, `remove` (text files in the app's private space, core 1.11), with a `done(error, value)` callback or as a `Promise` that rejects with an `Error` | `files.private` |
| `plinth:hub` | `listApps`, `appInfo`, `launch`, `setGrant`, `block`/`unblock`, `blockPublisher`/`unblockPublisher`, `pin`, `listGroups`, `createGroup`, `setGroup`, `remove`, `search`, `install`, `checkUpdates`, `update`, `lastError` (the Hub UI only, see [host-apis.md](host-apis.md)) | `hub.manage`, for a package that a trusted Hub key signed |

See [host-apis.md](host-apis.md) for how capabilities, denial, and the
manifest work together.

## Diagnostics

Every rejected feature has a stable code (SPEC.md §5.2, §15). The text
form matches `tsc`'s (`file(line,col): error PLnnnn: message`, with an
optional `help:` line); `--json` gives the same data as JSON. The
compiler's golden test suite
(`crates/plinth-compiler/tests/golden/`) has one example per code — see
`crates/plinth-compiler/tests/golden.rs` for the harness, and run it
with `PLINTH_BLESS=1` to regenerate the expected output after a message
change.

Codes are grouped by range:

| Range | Meaning |
|---|---|
| `PL1000`–`PL1007` | Modules and the closed world (SPEC.md §4.1): parse errors, the import graph (bare imports, missing modules, cycles), unknown std modules, a missing `app/main.tsx`, and undeclared capabilities (SPEC.md §11). |
| `PL2000`–`PL2024` | Rejected language features (SPEC.md §4.4): TypeScript/JavaScript syntax Plinth does not support yet or ever — `any`, classes (restricted subset), `this`, non-null assertions, type assertions, `for…in`, `delete`, generators and `await` in a `case` value, a typed or destructured `catch` variable, computed member access, advanced types, namespaces, `var`, user generics, getters/setters, labels, regular expressions, `BigInt`, non-reactive signal reads, unbound methods, and the class `extends`/`super`/`override` rules. |
| `PL3000`–`PL3013` | Types (SPEC.md §4, §4.3): the structural type checker — mismatches, unknown names/types, missing properties or fields, uncallable values, wrong argument counts, uninferable types, assigning to `const`, nullability, duplicate definitions, and missing `return`s. (`PL3011`, struct layout, is reserved but not emitted by any check today.) |
| `PL4000`–`PL4009` | JSX and the UI API (SPEC.md §6, §7.2): unknown controls, props, and children; missing required props; bad two-way bindings; the `app({...})` config; `navigate` targets; and image assets (missing, or an empty `alt`). |

