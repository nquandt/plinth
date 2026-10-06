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
| `number` | A 64-bit float (IEEE 754 `f64`). |
| `int` | A branded `number` from `plinth:core`. Literals and arithmetic on `int` stay `int` and wrap at 32 bits. `int` converts to `number` implicitly; convert back with `int(x)` (it truncates). |
| `boolean` | — |
| `string` | Immutable. `length`, indexing, and `slice` count UTF-16 code units, for TypeScript compatibility; see "Deviations" below. |
| `T[]` / `Array<T>` | A growable array, monomorphized per `T`. See `std/lib.d.ts` for the supported methods (`push`, `pop`, `map`, `filter`, `find`, `findIndex`, `some`, `every`, `forEach`, `includes`, `indexOf`, `slice`, `reverse`, `join`, `for…of`). |
| `Map<K, V>`, `Set<T>` | `K`/element type must be `string`, `int`, `number`, `boolean`, or an enum. Insertion order; lookup, insert, and delete are linear scans. `get` returns `null`, not `undefined`, for a missing key. |
| `T \| null` | Supported for every type except signals. `undefined` exists only as "absent" for optional properties and parameters; it is otherwise the same value as `null`. |
| interfaces, object types | A GC-allocated struct with a fixed layout. |
| classes | Single inheritance: fields, one constructor, methods, `new`, `extends`, `super(...)` as the first statement, `super.m()`, overriding, `instanceof` with narrowing. A base class must be declared before its subclasses. A method reference without a call (`arr.map(obj.method)`) is the error `PL2021`; write `() => obj.method()` instead. |
| union types | Tagged. Narrow with `typeof`, a discriminant field, `instanceof`, or `=== null`. |
| enums, string literal unions | Compile to `i32`/interned ids. |
| generics | Monomorphized: functions, interfaces, and type aliases. No constraints, no defaults, no conditional or mapped types. |
| `Promise<T>` | Reserved for `async`/`await`, not yet implemented (see "Not yet" below). |

**Status (current).** Implemented: `int`; `T | null` everywhere except
signals; discriminated unions with narrowing; generic functions,
interfaces, and type aliases; `Map`/`Set`; classes with single
inheritance and `instanceof`; `JSON.stringify`/`JSON.parse<T>`. **Not
yet:** static members, getters/setters, generic classes,
`async`/`await`, `try`/`catch`.

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
- `import`/`export` within the closed world (see below).
- Classes, single inheritance, `instanceof`, general union narrowing,
  generic functions and interfaces (all implemented; see "Status"
  above).

Planned for a later compiler version: `async`/`await`, `Promise.all`,
`try`/`catch`/`throw`.

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
- **Errors.** `throw` and uncaught runtime errors (null dereference,
  out-of-bounds index, out-of-memory) trap today; the host shows a
  "this app stopped" screen. `try`/`catch` is planned, pending the Wasm
  exception-handling proposal landing in every target runner.
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
| `plinth:core` | `int`, `Math`, `JSON.stringify`/`JSON.parse<T>`, `parseNumber`, `toString`, `console.log` (dev only) | none |
| `plinth:ui` | `signal`, `computed`, `effect`, `app()`, `navigate`, and every UI control (see [ui.md](ui.md)) | none |
| `plinth:time` | `now()`, `monotonicNow()`, `setTimeout`/`setInterval`/`clearTimeout`/`clearInterval` | none |
| `plinth:store` | `kv.get`/`kv.set`/`kv.remove`/`kv.keys`, `kv.lastError()` | `store.kv` |
| `plinth:clipboard` | `writeText`, `readText`, `lastError()` | `clipboard.write` / `clipboard.read` |
| `plinth:dialog` | `alert`, `confirm`, `prompt` (host-owned modal dialogs) | none |
| `plinth:net` | HTTP/WebSocket | `net:<host pattern>` — **in progress**, needs `async`/`await` |

See [host-apis.md](host-apis.md) for how capabilities, denial, and the
manifest work together.
