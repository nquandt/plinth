# Plinth — Specification

> Status: Draft v0.1 · 2026-10-05
> "Plinth" is a working codename. The CLI name (`plinth`), the module prefix (`plinth:`) and the package extension (`.plnt`) change with it.

---

## 1. Overview

Plinth is a framework for building cross-platform apps. Authors write apps in a strict subset of TypeScript with JSX. A compiler turns each app into a small WebAssembly artifact. A native host runtime runs that artifact and renders its UI with [`gpui-ce`](../gpui-ce).

Plinth has three central ideas:

1. **Build once, run anywhere.** One app package runs on Windows, macOS, Linux, the web, iOS, and Android.
2. **A semantic UI.** Apps declare *what* they need (screens, sections, controls, actions). The runtime decides *how* it looks and *where* it goes. All Plinth apps have the same look and feel, so users can move from app to app easily.
3. **A closed world.** Apps can only use the APIs that the Plinth runtime gives. There is no npm, no DOM, no Node, and no direct OS access. Every capability is declared, and the user grants it.

### 1.1 Goals

- Authors never write Rust, C, or any systems language.
- App artifacts contain **compiled Wasm only**. No embedded JS engine and no interpreter.
- One portable package format (`.plnt`) for all hosts.
- Optional single-file export: one executable (or one HTML file) per target.
- A consistent, accessible, adaptive UI with no per-app layout or styling code.
- A capability-based security model, suitable for a public app hub.
- First-class editor support through standard TypeScript tooling.

### 1.2 Non-goals

- Running arbitrary existing JS/TS code or npm packages.
- Pixel-level control over layout, color, or typography (except inside `<Canvas>`).
- Full ECMAScript semantics. Plinth TS is a statically typed subset with documented deviations.
- Being a web framework. The web is one host among six, not the model.

### 1.3 Relation to prior art

- **krate** (`C:\repos\krate`) is a reference, not a template. Plinth takes these ideas from it:
  - a zip-container package with a manifest,
  - declared capabilities with a rationale for each,
  - an import validator for Wasm artifacts,
  - a WIT-versioned host API,
  - a hub.

  Plinth does **not** take krate's authoring model (Rust compiled to Wasm) or its widget tree with author-controlled layout.
- **Svelte / Solid**: Plinth compiles JSX and reactivity away at build time, in the same way. There is no virtual DOM.
- **SwiftUI forms / platform HIGs**: these show that semantic, intent-based UI declarations can produce good layouts automatically.
- **AssemblyScript**: it shows that a TS-like language can compile AOT to Wasm with a linear-memory GC. Plinth does not use it, because its closure support is limited and it has no JSX.

---

## 2. Terminology

| Term | Meaning |
|---|---|
| **App** | A program written by an author in Plinth TS. |
| **Plinth TS** | The strict, statically typed TypeScript subset (§4). |
| **Artifact** | `app.wasm`, the compiled Wasm component of an app. |
| **Package** | A `.plnt` file: the artifact, the manifest, the assets, and an optional signature (§10). |
| **Host** | A native or web program that runs packages. |
| **Runtime** | The shared Rust code in every host: the Wasm runner, the UI runtime, the host APIs, and the policy. |
| **UI runtime** | The part of the runtime that turns the semantic tree into `gpui-ce` elements. |
| **Semantic tree** | The tree of controls that an app declares (§6). |
| **Op** | One instruction in the op buffer, from the guest to the host (§8). |
| **Capability** | A named permission that an app declares and the user grants (§11). |
| **Hub** | The app distribution service (§12). |

---

## 3. Architecture

```
                        AUTHOR MACHINE
 ┌──────────────────────────────────────────────────────────────────┐
 │ app/*.tsx ──► plinth compiler (Rust)                             │
 │                 oxc parse → check (Plinth TS) → lower JSX &      │
 │                 reactivity → IR → wasm-encoder → link runtime lib│
 │                 → wit-component wrap → validate imports          │
 │              ──► app.wasm + manifest.toml + assets ──► .plnt    │
 └──────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
                          HOST (one per target)
 ┌──────────────────────────────────────────────────────────────────┐
 │ Wasm runner adapter                                              │
 │   wasmtime (desktop, Android) │ AOT native (iOS) │ browser (web) │
 │        │ commit(ops)                 ▲ on-event(ev)              │
 │        ▼                             │                           │
 │ UI runtime: semantic tree ──► layout rules ──► control library   │
 │              (arena)          adaptive shell     on gpui-ce       │
 │                                                                  │
 │ Host API adapters (fs, net, store, clipboard, dialog, notify …)  │
 │        ▲ every call passes through the capability policy         │
 │ Package loader · signature check · consent UI · hub client       │
 └──────────────────────────────────────────────────────────────────┘
```

The design has three layers with stable contracts between them:

1. **Language → artifact.** The compiler owns this layer. The contract is the Plinth TS language and the `plinth:*` module typings.
2. **Artifact → host.** The contract is the WIT world `plinth:app/app@<ui-api>`. It holds the op-buffer format and the host APIs (§8).
3. **Host → platform.** Two parts are adapters:
   - The **Wasm runner adapter** runs the artifact on the platform.
   - The **host API adapters** give the platform services through one interface.

   Everything above these adapters is platform-independent.

---

## 4. Language: Plinth TS

Plinth TS is a strict, statically typed subset of TypeScript. **Every valid Plinth TS program is valid `.tsx`.** Thus `tsc`, the TypeScript language server, formatters, and editors work without changes. The Plinth compiler is the final authority. It rejects programs that use unsupported features, even when `tsc` accepts them.

### 4.1 Closed world

The closed world is enforced at three levels:

1. **The editor.** `plinth new` writes a `tsconfig.json` with `"types": []`, `"noLib": true` and `paths` that map `plinth:*` to the framework `.d.ts` files. `document`, `window`, `process`, `fetch`, and all other ambient globals do not exist, so they show as errors.
2. **The compiler.** Only two forms of import specifier are valid:
   - relative paths (`./x`, `../y`) inside the app,
   - `plinth:<module>` (framework modules) and, later, `hub:<publisher>/<lib>` (hub libraries compiled from Plinth TS).

   Any bare specifier (`"lodash"`, `"react"`) is a compile error. The compiler never reads `node_modules`.
3. **The artifact.** The validator rejects any Wasm import that is not in the declared `plinth:*` WIT world. It also rejects any `wasi:*` import. `plinth check` and the hub both run this validator.

### 4.2 Types

| Plinth TS | Representation | Notes |
|---|---|---|
| `number` | `f64` | |
| `int` | `i32` | A branded alias from `plinth:core`. Literals and arithmetic on `int` stay `i32`. Conversion to `number` is implicit. Conversion from `number` needs `int(x)`. |
| `boolean` | `i32` (0/1) | |
| `string` | GC ref to an immutable UTF-8 buffer | See §4.6 for the deviations. |
| `T[]` / `Array<T>` | GC ref to a growable vector | Monomorphized per `T`. |
| `Map<K,V>`, `Set<T>` | GC ref | `K` must be `string`, `int`, `number`, `boolean`, or an enum. |
| `T \| null` | nullable ref, or a tagged value for primitives | `undefined` exists only as "absent" in optional properties and parameters. It is otherwise the same value as `null`. |
| object types, interfaces | GC ref to a struct with a fixed layout | §4.4 |
| classes | GC ref to a struct + vtable | v0: no `extends`. v1: single inheritance. |
| union types | tagged representation | Narrowing with `typeof`, discriminant fields, `instanceof`, and `=== null`. |
| enums, string literal unions | `i32` | String literal unions compile to interned ids. |
| function types / closures | GC ref to (function index, environment) | |
| `Promise<T>` | GC ref to a task | §4.5 |
| generics | monomorphized | No higher-kinded tricks. Conditional and mapped types are rejected. |

### 4.3 Supported syntax (v0 → v1)

**v0 (the minimum to build real apps):**
- `let`, `const`, `function`, arrow functions, closures, default and optional parameters
- `if`/`else`, `switch`, `for`, `for…of` over arrays/maps/sets, `while`, `break`, `continue`, `return`
- arithmetic, comparison (`===`, `!==` only), logical ops, `??`, `?.`, the ternary operator, template literals
- object literals with a declared or inferred type, array literals, destructuring, spread into arrays and objects of known shape
- interfaces, type aliases, string literal unions, enums
- JSX with Plinth control tags only (§6)
- ES `import`/`export` within the closed world

**v1:**
- classes with single inheritance, `instanceof`
- general union types with narrowing
- `async`/`await`, `Promise.all`
- generic functions and generic interfaces
- `try`/`catch`/`throw` (see §5.6)

### 4.4 Rejected features

The compiler rejects these features, and each one has a clear error message and a suggested fix:

- `any`, `unknown` casts that bypass the checker, `as` casts between unrelated types, `!` non-null assertions on values the checker cannot prove
- `eval`, `new Function`, `with`, `arguments`, `this` outside class methods
- `==`, `!=`, `delete`, `in` on objects, `for…in`
- computed property access on objects without an index signature (`obj[key]` where `obj` is not a `Map` or an array)
- prototype access or changes, `Object.defineProperty`, `Proxy`, `Reflect`, `Symbol` (except the built-in iterator protocol)
- getters and setters (v0), decorators, namespaces, `declare` blocks in app code
- conditional types, mapped types, `infer`, template literal types, index access types
- ambient globals of any kind

**Structural typing.** TypeScript is structural. Plinth keeps structural *assignment* for object types and interfaces. When a value flows into an interface type and the compiler cannot prove the same layout, the compiler makes an **itable**: a per-(struct, interface) table of field offsets and method indices. Concrete struct types use direct field offsets. Interface-typed access is one extra indirection.

### 4.5 Async

`async` functions compile to state machines that live on the GC heap. A host API call that cannot finish at once returns a request id. The host later sends a `completion` event (§8.4), and the guest scheduler resumes the waiting task. There are no threads in the guest. All guest code runs on one logical thread, between events.

### 4.6 Documented deviations from ECMAScript

- **Number semantics:** `int` arithmetic wraps at 32 bits. `number` behaves as IEEE 754 `f64`.
- **Strings:** the runtime stores strings as UTF-8. `length`, indexing, and `slice` use UTF-16 code units, for compatibility with TS. They have an ASCII fast path. *(Open question Q5.)*
- **`null` and `undefined`** are one value at run time.
- **Object identity:** `===` on objects compares references, as in JS. There is no `==`.
- **Property order:** object literal fields have the declared order. Iteration over objects is not supported. Use `Map`.
- **Errors:** see §5.6.
- **`for (let i = …)`:** the loop variable is one binding for all iterations. Closures in the loop body see its last value. `for…of` variables and variables declared in a loop body get a new binding for each iteration, as in JS.
- **`toUpperCase` and `toLowerCase`:** the runtime maps ASCII, Latin-1, Latin Extended-A, Greek and Cyrillic. Other characters do not change. Multi-character mappings (`ß` → `SS`) are not done.
- **`parseNumber` (JS `Number(s)`):** the result is exact for up to 15 significant digits and a decimal exponent from -22 to 22. Other inputs can differ by one unit in the last place. Hex and binary literals are not parsed.
- **`toFixed`:** exact, except for integers above 2^64, which use the `toString` digits.

### 4.7 Standard library (`plinth:*` modules)

| Module | Contents |
|---|---|
| `plinth:core` | `int`, `Math`, `JSON` (typed `parse<T>` with a schema, and `stringify`), string helpers, `console.log` (dev only) |
| `plinth:ui` | Control components (§6.3), `signal`, `computed`, `effect`, `navigate`, `app()` entry |
| `plinth:store` | key-value store, simple SQL (SQLite) — capability `store.kv`, `store.sql` |
| `plinth:fs` | user-picked file tokens, app-private data directory — capability `fs.*` |
| `plinth:net` | `fetch`-like HTTP and WebSocket — capability `net:<host pattern>` |
| `plinth:time` | clock, timers, `sleep` |
| `plinth:clipboard`, `plinth:notify`, `plinth:share`, `plinth:dialog` | as the names say |
| `plinth:canvas` | 2D drawing API for `<Canvas>` — capability `ui.canvas` |
| `plinth:locale` | locale, number and date formatting, plural rules |

Each module has a hand-written `.d.ts` file in `std/` of this repo. The editor reads these files. The compiler has the same signatures built in, because it does not evaluate TypeScript type-level code. The test `std_typings_match` fails when the two disagree, so the `.d.ts` files stay the single source of truth.

In M1, `plinth:core` exports `Math`, `parseNumber`, `toString` and `console`; `plinth:ui` exports the reactive primitives, `app`, `navigate` and the M0 controls.

M2 adds three modules:
- `plinth:time`: `now()`, `monotonicNow()`, `setTimeout(f, ms)`, `setInterval(f, ms)`, `clearTimeout(id)` and `clearInterval(id)`. A timer id is a `number`. No capability is necessary.
- `plinth:store`: `kv.get(key): string | null`, `kv.set(key, value)`, `kv.remove(key)` and `kv.keys(): string[]`. Capability `store.kv`.
- `plinth:clipboard`: `writeText(text)` and `readText()`. Capabilities `clipboard.write` and `clipboard.read`.

A call to a module function whose capability is not in `plinth.toml` is the compile error `PL1007`.

---

## 5. Compiler

The compiler is written in Rust. It is one binary, `plinth`, together with the CLI (§13).

### 5.1 Pipeline

1. **Parse** with [`oxc`](https://github.com/oxc-project/oxc) (`oxc_parser`, `oxc_semantic`).
2. **Resolve** modules (closed world, §4.1).
3. **Check.** The Plinth type checker builds full types for every expression. It rejects unsupported features (§4.4). Its diagnostics use the same span format as `tsc`, so the editor can show them.
4. **Lower to HIR.** Desugar destructuring, spread, `?.`, `??`, `for…of`, and template literals.
5. **Lower JSX and reactivity** (§7). This step turns templates into "create once, bind slots" code.
6. **Lower to MIR.** Monomorphize generics, convert closures, transform async functions into state machines, compute struct layouts and itables.
7. **Codegen** with `wasm-encoder` into a core Wasm module (linear memory).
8. **Link** the prebuilt **runtime library** (`plinth-rt`, Rust compiled to `wasm32-unknown-unknown`). It holds the GC, strings, collections, signals, the op-buffer writer, and the scheduler. The compiler embeds `plinth-rt`. The **append linker** adds the app's types, functions, globals, table entries and passive data segments after those of `plinth-rt`, so no index inside the runtime changes. App code calls the exported `__plinth_rt_*` functions (`rt_abi.rs` lists them; the linker checks each name and type). The runtime calls app code only through *callables*: a thunk in the function table and a closure object. A start function gives the app entry point to the runtime. The linker removes the `__plinth_rt_*` exports. It also replaces each runtime function that the app cannot reach with a stub (`unreachable`). A stub keeps its index, so the append rule stays true. The roots are the runtime exports, the start function, the table entries, and the runtime functions that the app code calls.
9. **Wrap** the module as a component with `wit-component`, against the `plinth:app` WIT world.
10. **Validate** the imports (§4.1 level 3) and optimize the module (`wasm-opt`, optional).

### 5.2 Diagnostics

- The compiler stops at the first file with errors only when a later phase cannot continue. Otherwise it collects all errors.
- Each rejected feature gets a stable error code (`PL1001` …), a message, and, where possible, a suggestion. Example: "use a `Map<string, T>` instead of an index access".
- `plinth check --json` gives machine-readable output for the editor and the hub.

### 5.3 Editor integration

- **v0:** standard `tsc`/tsserver with the closed-world `tsconfig`, plus `plinth check --watch` for Plinth-specific errors.
- **v1:** a tsserver plugin that runs the Plinth checker and shows `PL*` diagnostics inline.

### 5.4 Memory and GC

- **Linear memory, not Wasm GC.** The iOS host must translate Wasm to native code ahead of time (§9.2). The AOT tools (`wasm2c`, WAMR AOT) have limited Wasm GC support, or none. Linear memory runs the same way in wasmtime, in every browser, and through AOT. A Wasm GC backend can come later as a second target.
- **Collector:** a non-moving mark-sweep collector in `plinth-rt`. The memory allocator under it has size-class free lists for small blocks and a first-fit list for large blocks. Compiler-defined types (structs, closure environments) register their size and reference offsets in a type table at start.
- **Closures:** a variable that a nested function uses lives in a heap frame: one frame per call, and one per loop iteration for variables declared in a loop body. Other variables are Wasm locals.
- **Key simplification:** the GC runs **only when the guest is idle**, that is, after `on-event` (or `init`) returns and the Wasm stack is empty. Thus the GC has no stack roots. The only roots are the globals, the signal graph, the pending tasks, and the live node handles. The compiler does not need a shadow stack.
- **Safety valve:** if one event handler allocates past a hard limit, the guest traps with an `OutOfMemory` error. This rule stops runaway loops from taking down the host.

### 5.5 Code size targets

| Item | Target |
|---|---|
| `plinth-rt` linked into each artifact | ≤ 60 KiB (after the linker stubs the unreachable functions; `wasm-opt -Oz` is optional). The raw `plinth-rt` has no strict limit: an app pays only for the runtime functions that it reaches. |
| "Counter" app artifact | ≤ 80 KiB |
| Typical small app (5 screens) | ≤ 300 KiB |

### 5.6 Errors and exceptions

- **v0:** `throw` and uncaught runtime errors (null dereference, out-of-bounds index, OOM) **trap**. The host catches the trap and shows a standard "This app stopped" screen with a report option. It keeps the guest's last committed tree in a read-only state.
- **v1:** `try`/`catch` through the Wasm exception-handling proposal, after all target runners support it (wasmtime, browsers, the iOS AOT path). Until then, `plinth:core` gives `Result<T, E>` for recoverable errors, and all fallible host APIs return it.

---

## 6. Semantic UI model

### 6.1 Principles

1. **Intent, not appearance.** Apps declare controls, structure, roles, and data. They cannot set position, size, margin, padding, color, font, or border.
2. **The runtime owns layout and navigation.** The runtime chooses layouts from the tree and the form factor.
3. **Every control is accessible.** Each interactive control requires a `label`. The runtime maps every node to an AccessKit role.
4. **Adaptive by default.** The same tree renders differently by window width class (`compact` < 600 px ≤ `regular` < 1200 px ≤ `wide`) and by input mode (touch or pointer).
5. **One escape hatch.** `<Canvas>` gives a sandboxed drawing surface. It is a declared capability.

### 6.2 App structure

```tsx
import { app, Screen } from "plinth:ui";
import Home from "./home";
import Settings from "./settings";

export default app({
  accent: "teal",                      // one of the runtime's named accent colors
  screens: {
    home:     { title: "Home",     icon: "house",  component: Home },
    settings: { title: "Settings", icon: "gear",   component: Settings },
  },
  primary: ["home", "settings"],       // top-level destinations
});
```

**Navigation shell:** the runtime chooses it from the width class.

| Width class | Shell |
|---|---|
| `compact` | Bottom tab bar for `primary`, with a stack navigator inside each tab. |
| `regular` | Navigation rail and a stack. |
| `wide` | Sidebar and content. Optional list-detail split when a screen declares `detail`. |

`navigate("settings")`, `navigate.push(Screen, props)`, and `navigate.back()` are the only navigation APIs.

**Status (UI API 1.2):** `navigate("name")` selects a primary screen. `navigate.push("name")` pushes any declared screen on the stack of the current primary tab, and `navigate.back()` pops it. The compiler checks each name against `app({ screens })`. `push` does not take props yet. Screens that are not in `primary` are pushable only. The host keeps one stack for each primary tab. It shows a back control in the screen header when the stack has more than one entry, and Escape or Alt+Left goes back. The guest marks the primary screens with the `navigate` op kind `mark-primary` (4) at start.

### 6.3 Control vocabulary (UI API 1.0)

Every control is a typed JSX intrinsic from `plinth:ui`. Props in **bold** are required.

**Structure**

| Control | Props | Notes |
|---|---|---|
| `Screen` | **title**, `actions?: Action[]`, `detail?` | The root of each screen. Actions go to the toolbar or an overflow menu. |
| `Section` | `title?`, `footer?` | A titled group. The runtime decides card or inset-group style. |
| `Group` | `axis?: "auto" \| "row" \| "column"` | A logical grouping (UI API 1.1). In a row, each child gets the same width. `auto` is a row on `regular` and `wide`, and a column on `compact`. The runtime owns the spacing. |
| `Tabs` | **items** | In-screen segmented tabs. |
| `Sheet` | **open**, **title**, `onClose` | Modal on compact, side panel or dialog on wide. |
| `Dialog` | **open**, **title**, `actions` | Alert or confirmation. |

**Collections**

| Control | Props | Notes |
|---|---|---|
| `List` | **items**, **key**, **row** (render fn), `onSelect?`, `swipeActions?`, `empty?` | Virtualized. Keyed diffing is compiled (§7.3). |
| `Row` | **title**, `subtitle?`, `leading?: Icon \| Image`, `trailing?: Text \| Badge \| Toggle`, `onPress?` | The standard list row. |
| `Grid` | **items**, **key**, **cell** | Responsive column count. |

**Content**

| Control | Props |
|---|---|
| `Text` | `style?: "body" \| "caption" \| "mono"`, `tone?: "default" \| "muted" \| "danger" \| "success"`, `align?: "start" \| "center" \| "end"` (1.1) |
| `Heading` | `level?: 1 \| 2 \| 3`, `align?` (1.1) |
| `Image` | **src** (asset or URL token), **alt**, `aspect?` |
| `Icon` | **name** (from the runtime icon set) |
| `Badge` | **text**, `tone?` |
| `Progress` | `value?` (missing means indeterminate), **label** |
| `Empty` | **title**, `message?`, `action?` |

**Inputs**: each input uses a `value` signal (two-way) or `value` plus `onChange`.

| Control | Props |
|---|---|
| `Button` | **label** (or a text child), `role?: "primary" \| "default" \| "destructive"`, `size?: "default" \| "large"` (1.1), **onPress**, `disabled?`, `icon?` |
| `TextField` | **label**, **value**, `placeholder?`, `kind?: "text" \| "email" \| "password" \| "url" \| "search"`, `error?` |
| `TextArea` | **label**, **value**, `placeholder?` |
| `NumberField` | **label**, **value**, `min?`, `max?`, `step?` |
| `Toggle` | **label**, **value** |
| `Checkbox` | **label**, **value** |
| `Picker` | **label**, **value**, **options** | 
| `Slider` | **label**, **value**, `min`, `max`, `step?` |
| `DatePicker` | **label**, **value**, `mode?: "date" \| "time" \| "datetime"` |

**Actions**

| Control | Props |
|---|---|
| `Action` (value, not a tag) | **label**, **onPress**, `icon?`, `role?`, `shortcut?` |
| `Menu` | **label**, **actions** |

**Escape hatch**

| Control | Props |
|---|---|
| `Canvas` | **label**, **draw** (`(ctx: Canvas2D, size) => void`), `onPointer?`, `aspect?` |

UI API 1.1 adds `Group`, `Button.size` and `Text`/`Heading` `align`. These props state intent (a row of equal keys, a large key, a numeric display that reads from the end); they do not give pixels, spacing or colors.

UI API 1.2 adds the controls `Checkbox`, `TextArea`, `Slider`, `NumberField`, `Picker`, `Progress`, `Badge`, `Tabs`, `Sheet`, `Dialog`, `Menu`, `Grid` and `Action`, the `Screen` prop `actions`, and 20 more icon names. These rules apply in 1.2:

- `Slider` and `NumberField` bind a `Signal<number>` both ways.
- `Picker.options` and `Tabs.items` must be array literals of string literals. The compiler joins them with U+001F into one string prop. A `Picker` with four options or less shows as a segmented control, and with more as a list.
- `Action` is a tag in 1.2. `actions={[<Action ... />]}` takes an array literal of `Action` elements. The elements become child nodes of the `Screen`, `Dialog` or `Menu`, and the host tells them apart by their kind. The toolbar shows two actions on `compact` and four on `regular` and `wide`; the rest go into an overflow menu.
- A destructive `Action` asks for confirmation through a host dialog, unless it has `confirm={false}`.
- `Sheet` and `Dialog` bind `open` both ways. When the user closes one (backdrop, close button, Escape), the host sets `open` to false and `Sheet` sends `close`.
- `Grid` uses the keyed reconciler of `List`. The runtime sets the column count from the width class (2, 3 or 4).
- `Progress` without `value` is indeterminate.
- `Menu` and the toolbar overflow open an anchored popover above the content. It closes on an outside click, on Escape, and after an action runs. Its AccessKit roles are `menu` and `menu item`.
- `Slider` also follows pointer drag on its track, snapped to `step`.
- Every control has an AccessKit role, name and state. Interactive controls take keyboard focus (Tab) and activation (Enter or Space; arrows for `Slider`).

### 6.4 Layout rules (runtime-owned)

- Forms: a vertical stack of fields. Labels go above the fields on `compact` and are aligned beside them on `regular` and `wide`.
- Spacing comes only from runtime tokens (`space.1` … `space.6`). Apps cannot reach them.
- Width: content columns have a maximum readable width on `wide` and are centered.
- `Section`s in a screen flow vertically. On `wide`, the runtime can place independent sections in two columns.
- Button groups: the primary action goes last on desktop and first on mobile, as each platform's convention says.
- Destructive actions always ask for confirmation through a standard dialog, unless the app sets `confirm: false` on the `Action`.

### 6.5 Theme

- The runtime supplies the type scale, color tokens, elevation, corner radii, and motion, in light and dark.
- Each app chooses one **accent** from a fixed named palette. No other theming exists.
- The runtime respects the OS settings for dark mode, contrast, reduced motion, and text size.

### 6.6 Versioning

- The control set is **UI API** `MAJOR.MINOR`. A minor version adds controls or optional props. A major version can remove or change them.
- Each package declares `ui-api = "1.0"`. A host supports a range, for example `1.0–1.4`. The hub uses this range for compatibility checks.
- Unknown props are a compile error. A host that gets an unknown control kind (a version mismatch) renders a placeholder and logs an error. It does not crash.

---

## 7. Reactivity and JSX compilation

### 7.1 Primitives (`plinth:ui`)

```ts
const count = signal(0);            // Signal<int>: count() reads, count.set(v) / count.update(f) write
const double = computed(() => count() * 2);
effect(() => console.log(count()));  // dev-only logging example
```

- Reads inside a tracking scope (a template slot, `computed`, or `effect`) record dependencies.
- Writes mark dependents dirty. The scheduler flushes all updates **once per event**, after the handler returns.
- A flush writes the minimal set of ops (`set-prop`, `insert`, `remove`) to the op buffer, then calls `commit` one time.

### 7.2 JSX lowering

The compiler turns each component function into:

1. **A create function** that emits `create` and `insert` ops for the static structure, one time.
2. **Slot bindings.** For each dynamic prop or child, the compiler emits an effect that writes `set-prop` (or a child update) when its dependencies change.
3. **Event bindings.** For each `on*` prop, it registers a handler id and emits a `listen` op.

Components are plain functions that run one time per instance. Props that are signals stay reactive. There is no re-render of a whole component and no virtual DOM.

### 7.3 Lists

`<List items={todos} key={t => t.id} row={t => <Row title={t.title} />} />` compiles to a keyed reconciler in `plinth-rt`. It compares the old and new key arrays and emits `insert`, `move`, and `remove` ops. Each row is its own reactive scope. The host virtualizes the list, so it requests row content only for visible rows. *(Open question Q6: guest-side or host-side virtualization.)*

---

## 8. Guest ↔ host ABI

### 8.1 WIT world

The WIT files are the source of truth. They live in `wit/plinth/` in this repo.

```wit
package plinth:app@1.0.0;

world app {
  // UI
  import ui: interface {
    commit: func(ops: list<u8>);           // one batched op buffer per flush
  }
  // Host APIs (each gated by a capability)
  import store;  import fs;  import net;  import time;
  import clipboard;  import notify;  import dialog;  import locale;  import canvas;

  export init: func(args: list<u8>);       // build the initial tree
  export on-event: func(ev: list<u8>);     // one or more events, batched
}
```

**`init` args:** a sequence of records (`tag: u8`, `len: u32`, `len` bytes). Tag 1 asks for GC stress mode (tests only: collect after every call and poison freed memory). Tag 2 carries a hot-reload snapshot (dev only, §13). An empty buffer is a normal start, and a guest skips unknown tags.

The artifact is a **Wasm component**.
- **Desktop and Android:** wasmtime runs the component directly.
- **Web:** the build creates browser glue with `jco` (§9.2).
- **iOS:** the build lowers the component to a core module and translates it AOT (§9.2).

### 8.2 Op buffer format (UI API 1.0)

The buffer is little-endian. It is a sequence of ops, and each op starts with a `u8` opcode. Node ids are `u32`s that the guest assigns. Id `0` is the "no node" value.

| Op | Code | Payload |
|---|---|---|
| `create` | `0x01` | `id: u32`, `kind: u16` (control kind) |
| `remove` | `0x02` | `id: u32` (removes the subtree, and the host frees its ids) |
| `insert` | `0x03` | `parent: u32`, `id: u32`, `before: u32` (`0` means append) |
| `move` | `0x04` | `parent: u32`, `id: u32`, `before: u32` |
| `set-prop` | `0x05` | `id: u32`, `prop: u16`, `value: Value` |
| `listen` | `0x06` | `id: u32`, `event: u16`, `handler: u32` |
| `unlisten` | `0x07` | `id: u32`, `event: u16` |
| `set-root` | `0x08` | `screen: u32`, `id: u32` |
| `navigate` | `0x09` | `kind: u8` (push, replace, back, select-primary), `screen: u32`, `args: Value` |
| `text` | `0x0A` | `id: u32`, `value: Value(str)` (a shortcut for text content) |

`Value` is a `u8` tag followed by data:

| Tag | Type | Data |
|---|---|---|
| `0` | null | — |
| `1` | bool | `u8` |
| `2` | int | `i32` |
| `3` | number | `f64` |
| `4` | string | `len: u32`, UTF-8 bytes |
| `5` | enum | `u16` (a prop-specific enum value) |
| `6` | list | `count: u32`, `Value`× |
| `7` | handle | `u32` (asset, image, or file token) |

The `kind`, `prop`, `event`, and enum ids are generated from one table in `wit/plinth/ui-api.toml`. The compiler and the host both read this table, so they cannot disagree.

### 8.3 Ordering and atomicity

- The host applies each `commit` atomically. A frame never shows half of a commit.
- Ops in one commit are applied in order.
- The guest never reads the host tree. The guest is the source of truth for structure, and the host is the source of truth for layout and pixels.

### 8.4 Event format

`on-event` gets a buffer of one or more events:

| Event | Code | Payload |
|---|---|---|
| `ui` | `0x01` | `handler: u32`, `event: u16`, `value: Value` (for example the new text or the toggle state) |
| `completion` | `0x02` | `request: u32`, `result: Value` (async host call result) |
| `timer` | `0x03` | `timer: u32` |
| `lifecycle` | `0x04` | `kind: u8` (foreground, background, low-memory, before-quit) |
| `visible-rows` | `0x05` | `list: u32`, `from: u32`, `to: u32` (if host-side virtualization asks for rows) |

**Two-way bindings** (`TextField value={sig}`): the host updates the displayed control at once and sends a `ui` event. The guest updates the signal. It does **not** echo a `set-prop` back to the host unless the value differs (for example, input filtering). This rule stops the cursor from jumping and stops feedback loops.

### 8.5 Host APIs

- Each host API module is a WIT interface. Fallible calls return `result<T, error>`.
- Calls that can block (net, file dialogs, large reads) return a `request: u32` at once and finish through a `completion` event. Synchronous calls are permitted only when they are guaranteed fast (clock, locale, kv get from cache).
- Every call checks the capability policy (§11). A denied call returns `error.denied(reason)` and never traps.
- **Status (M2):** `wit/plinth/app.wit` has the interfaces `error` (`host-error`, `denied-reason`), `time` (`now`, `monotonic-now`, `set-timer`, `cancel-timer`), `store` (`kv-get`, `kv-set`, `kv-delete`, `kv-keys`) and `clipboard` (`write-text`, `read-text`). All of these calls are synchronous. The host fires timers with the `timer` event. The desktop host polls the timers each 15 ms and delivers one firing for each poll. It does not catch up missed ticks. The kv store is a JSON file at `%APPDATA%\plinthpps\<id>\kv.json`.

---

## 9. Host runtime

### 9.1 Crates (Rust, in this repo)

| Crate | Role |
|---|---|
| `plinth-runtime` | Package loading, the policy, the event loop glue, and the host API dispatch. It is platform-independent. |
| `plinth-ui` | The semantic tree arena, the op decoder, the layout rules, the adaptive shell, and the control library on `gpui-ce`. |
| `plinth-runner-wasmtime` | A Wasm runner adapter for desktop and Android. |
| `plinth-runner-web` | A Wasm runner adapter that uses the browser `WebAssembly` API and `jco` glue. |
| `plinth-runner-aot` | Support for iOS. It builds the app module into native code at package time. |
| `plinth-host-api` | Traits for each host API, with one implementation module per platform. |
| `plinth-host-{windows,macos,linux,web,ios,android}` | Thin binaries and entry points for each platform. |

### 9.2 Wasm runner adapters

```rust
trait WasmRunner {
    fn instantiate(&mut self, artifact: &Artifact, imports: HostImports) -> Result<GuestHandle>;
    fn call_init(&mut self, g: &GuestHandle, args: &[u8]) -> Result<()>;
    fn call_on_event(&mut self, g: &GuestHandle, events: &[u8]) -> Result<()>;
    fn memory_stats(&self, g: &GuestHandle) -> MemoryStats;
}
```

| Target | Runner | Notes |
|---|---|---|
| Windows, macOS, Linux | `wasmtime` + Cranelift | Cache compiled code on disk per artifact digest. |
| Android | `wasmtime` + Cranelift | Android permits JIT. |
| iOS | AOT: `wasm2c` (or WAMR AOT) → C → linked into the IPA | iOS does not permit JIT. Thus each iOS build includes its app as native code. The hub serves iOS apps as built IPAs or through an approved container app. *(Open question Q3.)* |
| Web | browser `WebAssembly` | The `gpui_web` host module and the app module run as two Wasm instances on one page. The connection uses `jco` glue that the build creates. |

**Fuel and limits:** each runner enforces a time limit for each event (wasmtime epoch interruption on desktop) and a memory cap. If a handler is too slow, the host interrupts it and shows the standard "app is not responding" UI.

### 9.3 UI runtime on `gpui-ce`

- One gpui `Entity<PlinthRoot>` per window implements `Render`.
- `PlinthRoot` holds the semantic tree in an arena (`Vec<Node>` + a free list, indexed by the guest `id`).
- Each frame, `render()` walks the visible tree and calls the **ElementAdapter** for each node kind:

  ```rust
  trait ControlAdapter {
      fn kind(&self) -> ControlKind;
      fn render(&self, node: &Node, cx: &mut RenderCx) -> AnyElement;
  }
  ```

  A registry maps `ControlKind → Box<dyn ControlAdapter>`. Hosts can override single adapters for platform-specific behavior, for example a native date picker on mobile.
- The layout rules (§6.4) and the shell (§6.2) are Rust code in `plinth-ui`. They use `gpui-ce` styling (taffy) inside the runtime only.
- **Control library:** first, evaluate [`gpui-component`](https://github.com/longbridge/gpui-component) for compatibility with the `gpui-ce` fork. If it fits, wrap its controls. If it does not fit, write the controls in `plinth-ui`.
- Accessibility: every node maps to an AccessKit role through the `gpui-ce` a11y hooks (`a11y_role`, `write_a11y_info`).

### 9.4 Host API adapters

```rust
trait Store { fn kv_get(&self, app: &AppId, key: &str) -> Result<Option<Vec<u8>>>; /* … */ }
trait Net   { fn request(&self, req: HttpRequest, done: Completion); /* … */ }
// one trait per plinth:* module
```

Each platform crate gives the implementations. A missing implementation returns `error.unsupported`. The manifest validator warns at build time when an app declares a capability that a target does not support.

### 9.5 Platform matrix

| Target | gpui-ce backend | Runner | Status |
|---|---|---|---|
| Windows | Win32 + DirectX (`gpui_windows`) | wasmtime | Available in the fork |
| macOS | Metal (`gpui_macos` / `gpui_apple`) or wgpu | wasmtime | Available in the fork |
| Linux | Wayland / X11 (`gpui_linux`) + wgpu | wasmtime | Available in the fork |
| Web | `gpui_web` (WebGPU / WebGL) | browser | Available in the fork. Threads need COOP/COEP headers. `single_threaded_web()` exists. |
| iOS | — | AOT | **No backend in `C:\repos\gpui-ce`.** It needs the mobile fork (Q1). |
| Android | — | wasmtime | **No backend in `C:\repos\gpui-ce`.** It needs the mobile fork (Q1). |

---

## 10. Package format

### 10.1 `.plnt` container

A `.plnt` file is a zip archive with these entries:

```
plinth-profile          # one line: "plinth/1"
manifest.toml
app.wasm                # the app module: only the app code (§10.4)
assets/…                # read-only resources (images, data, fonts are NOT allowed: the runtime owns typography)
signature.json          # optional: publisher signature over the digests of all other entries
source/…                # optional: app sources, for "open as project"
```

The loader rejects path traversal, absolute paths, symlinks, duplicate entries, and archives that expand too much (zip bombs). It checks that `manifest.entry` matches `app.wasm`. Opening a package grants nothing.

### 10.2 Manifest

`runtime` names the core version that the app needs, for example `plinth-core/1.0` (§10.5).

```toml
id        = "com.example.notes"
name      = "Notes"
version   = "1.2.0"
publisher = "example"
ui-api    = "1.0"
runtime   = "plinth-core/1.0"
entry     = "app.wasm"
accent    = "teal"
icon      = "assets/icon.png"

[[capabilities]]
name      = "store.kv"
rationale = "Save your notes on this device."

[[capabilities]]
name      = "net:api.example.com"
rationale = "Sync notes with your Example account."

[targets]
exclude = []            # e.g. ["ios"] if a capability is unsupported there
```

### 10.3 Single-file export

`plinth build --target <t> --single-file` makes:

| Target | Output |
|---|---|
| Windows | `App.exe`: the host stub with the `.plnt` added as a trailing payload (signed with Authenticode if a certificate is configured) |
| Linux | `app`: one ELF with the trailing payload (optional AppImage) |
| macOS | `App` single binary, or `App.app` when signing or notarization is required |
| Web | `app.html`: the host and app wasm inline (base64 or a compressed blob). If threads are on, the output is `index.html` + `.wasm` files, because of the COOP/COEP needs. |
| iOS | `.ipa` (AOT-compiled app inside) |
| Android | `.apk` / `.aab` |

The stub finds its payload through a footer: the magic bytes `PLNTH\0`, the offset, the length, and a digest.

### 10.4 App modules and the runtime

**A `.plnt` is built one time and runs everywhere.** It holds only app code and is the same file for every OS and device. The runtime (`plinth-rt`) belongs to the host. Each user installs a host one time: the `plinth` CLI (`plinth run app.plnt`) or, later, a Plinth runner app. A native export (`plinth native`, §10.3) is the opposite case: it is specific to one host platform and contains the runtime.

- `app.wasm` is a core Wasm module. It imports from the module `plinth-rt`: the runtime functions that it calls (the names in `rt_abi`, without the `__plinth_rt_` prefix), the immutable `i32` global `table_base`, `memory` and `table`. It has no exports, no own memory or table, and only passive data.
- **No dependency on one runtime build.** The app uses runtime functions only by name and type. Its table indices are relative to `table_base`, and its one element segment starts at `table_base`. It contains no runtime index, size or build id.
- **Core version.** The custom section `plinth-core` holds the core version that the app needs, `MAJOR.MINOR` (for example `1.0`). The manifest field `runtime` is `plinth-core/1.0`. §10.5 tells how a host picks a core.
- **Compatibility rule.** Inside one major version, a core only adds functions and constants, and each addition increments the minor version. A core never removes a function, changes its type or its behavior, or changes the object layout (`HEADER`, `STR_BYTES`, the built-in type ids). A breaking change makes a new major version.
- **Load.** The host picks a core (§10.5), checks each import against the exports of that core (name and type), remaps the app indices into the index spaces of its runtime, replaces `global.get table_base` with its own table size, and uses the append linker (§5.1 step 8) to make one module. Then it runs that module. The link takes some milliseconds. A host can cache the result by the digest of the app.
- **Security.** The app can reach only the runtime functions. All host APIs go through the runtime and the capability policy (§11).
- **Sizes (2026-10-05).** The counter app module is about 1.8 KB and its package is 2 KiB. The todo app module is about 5 KB.

The compiler runs the same load step after each build, so each build tests it.

### 10.5 Runtime cores and versions

A **core** is one build of `plinth-rt`. It is a Wasm module, so a core version is the same file on every host platform. A `.plnt` that needs core 1.3 behaves the same on Windows, macOS, Linux, the web and mobile, because each of these hosts runs the same core 1.3 file.

- A core holds its version in the custom section `plinth-core` (`MAJOR.MINOR`).
- A host keeps more than one core, as a version manager (for example `nvm`) does: the core that it was built with, and the cores in its cores directory (`%LOCALAPPDATA%\plinth\cores\<version>\core.wasm` on Windows, `$XDG_DATA_HOME/plinth/cores/...` on other systems, or `PLINTH_CORES_DIR`).
- **Selection.** For each app, the host picks the core with the same major version and the highest minor version that is not lower than the app's minor version. If there is no such core, the host tells the user which core to install.
- **Backward compatibility.** Because a core only adds functions inside its major version, the newest core of a major version runs every older app of that major version. A breaking change makes a new major version, and the old and the new core stay installed side by side.
- **The native part of a host** (the renderer and the host APIs) must also support the WIT world that a core imports (`plinth:app@1.0.0`). A new core major version that needs a new WIT world also needs a host that supports it.
- **Commands:** `plinth core list`, `plinth core install <core.wasm>`, `plinth core export <file>`. Later, the hub and npm distribute cores.

---

## 11. Security and capabilities

- **Default deny.** A package can do nothing outside its own UI until the user grants its declared capabilities.
- **Capability names** (v0): `store.kv`, `store.sql`, `fs.pick` (user-picked files, delivered as tokens, never paths), `fs.app-data`, `net:<host-pattern>`, `clipboard.read`, `clipboard.write`, `notify`, `ui.canvas`, `ui.window.multi`, `camera`, `microphone`, `location` (later).
- Each capability in the manifest must have a `rationale`. The consent screen shows it.
- Every host API call passes through `Policy::check(app, capability)`. Denial reasons are distinct: `undeclared`, `refused`, `unsupported`.
- Grants are stored per app id and per publisher key. A new version that asks for new capabilities asks the user again.
- App data is isolated per app id. Apps cannot read the data of other apps.
- Network requests go only to hosts that the `net:` patterns allow. The host also blocks requests to private network ranges, unless a separate `net.local` capability allows them.

---

## 12. App hub

- **Publishing:** `plinth publish` uploads a signed `.plnt`. The hub runs these checks:
  1. the import validator,
  2. the manifest checks,
  3. a UI API compatibility check,
  4. an automatic screenshot run (the runtime renders each declared screen headless),
  5. an accessibility lint (labels, contrast is runtime-owned).
- **Install:** the user installs the Plinth runtime one time. It contains a hub client. Apps then install as packages of some KB. The user can also open `.plnt` files directly.
- **Updates:** the hub checks versions against the `runtime` and `ui-api` ranges. The runtime updates itself separately.
- **Libraries (later):** `hub:<publisher>/<lib>` packages are Plinth TS source libraries that the same compiler builds. They have the same closed-world rules and are versioned and locked in `plinth.lock`.
- **Identity:** each publisher has a signing key. The hub can co-sign. The runtime shows the verified publisher on the consent screen.

---

## 13. Tooling (CLI)

| Command | Purpose |
|---|---|
| `plinth new <name>` | Make a project with `tsconfig.json`, `plinth.toml`, `app/main.tsx`, and the typings. |
| `plinth dev [--target desktop\|web]` | Compile, run in a dev host, and hot-reload on save. Hot reload keeps signal state where the shapes did not change. |
| `plinth check [--json] [--watch]` | Type check and run the closed-world rules. No build. |
| `plinth build [--target t] [--single-file] [--release]` | Make the `.plnt` package, or a single-file export. |
| `plinth run <file.plnt\|url>` | Run a package in the local runtime. |
| `plinth validate <file.plnt>` | Validate the package structure, the manifest, and the Wasm imports. |
| `plinth publish` | Sign and upload to the hub. |
| `plinth shoot <file.plnt>` | Render each screen headless to PNGs (for review and tests). Today this is the separate dev tool `plinth-shoot <file.plnt> <out-dir>`, because it needs the `test-support` features of gpui-ce. It uses the headless WGPU renderer and writes one PNG for each screen and each width class. |

**Hot reload (status).** `plinth dev` builds with a dev core (`plinth-rt` with its `dev` feature). Each module-level `signal(...)` gets a stable key (module and name) and a shape code (`number`, `int`, `boolean`, `string`, or one of these with `| null`). Before the host starts a new build, it sends a snapshot request through `on-event`, and the old instance answers with a `snapshot` op. The host gives the snapshot to `init(args)` of the new instance, which restores each signal whose key and shape match. Other signals start with their new initial values. A signal declared in a component or in a `List` row is kept too: its key is the function name and the position of the signal in that function, combined with the instance path of its scope (a row's path comes from its row key). Sibling rows keep their own state, and the state of a removed row is dropped. Signals with arrays, objects, `Map` or `Set` values are not kept yet. The selected screen and the navigation stack of each primary tab stay when their screens still exist. Release builds contain none of this code.

### 13.1 Distribution through npm

The developer experience must be as simple as `npm create vite` or `npx sv create`.

- **Start a project:** `npm create plinth@latest my-app` (the `create-plinth` package, or `plinth new my-app`) writes a "hello world" app, the closed-world `tsconfig.json`, `plinth.toml`, and a `package.json`.
- **Daily commands** are npm scripts: `npm run dev`, `npm run build`, `npm run check`. Each script calls the `plinth` binary.
- **The `@plinth/cli` package** gives the `plinth` command and the `plinth:*` typings. (The npm name `plinth` is taken; see Q2.) The binary comes in one optional package for each platform (as `esbuild` and `@biomejs/biome` do), so `npm install` gets no build step and needs no Rust toolchain.
- **One self-contained binary.** The `plinth` binary contains the compiler, the prebuilt `plinth-rt`, the typings, and the desktop dev host. It never reads `node_modules` to compile. Only the editor reads the typings there.
- **The project tsconfig** maps `plinth:*` to `.plinth/types/*`. Each `plinth` command writes the typings of its own version there, so the editor and the compiler always agree, with or without npm.
- **Packing:** `node scripts/npm-pack.mjs` makes the tarballs in `target/npm` from a release build. In CI, `--platform <os>-<arch> --binary <path>` packs one platform package, and `--meta` packs `@plinth/cli` and `create-plinth`. `scripts/set-version.mjs` writes one version into `Cargo.toml` and all `package.json` files.
- **Release:** `.github/workflows/release.yml` builds the binary for `win32-x64`, `linux-x64`, `darwin-x64` and `darwin-arm64` on a `v*` tag, and publishes the packages with the `NPM_TOKEN` secret. `docs/RELEASING.md` gives the steps. Only the Windows build is tested.

**Project layout (an app):**

```
my-app/
  plinth.toml         # app manifest source (becomes manifest.toml)
  tsconfig.json       # closed-world config, generated
  app/
    main.tsx          # export default app({...})
    home.tsx
  assets/
  .plinth/types/      # plinth:* .d.ts files, generated
```

---

## 14. Repository layout (this repo)

```
plinth/
  SPEC.md
  wit/plinth/              # WIT world + ui-api.toml (single source of truth for ids)
  crates/
    plinth-cli/            # the `plinth` binary
    plinth-compiler/       # parse, check, lower, codegen
    plinth-rt/             # guest runtime library (→ wasm32-unknown-unknown)
    plinth-runtime/        # host core: package, policy, dispatch
    plinth-ui/             # semantic tree + controls on gpui-ce
    plinth-host-api/       # host API traits + platform impls
    plinth-runner-wasmtime/
    plinth-runner-web/
    plinth-runner-aot/
    plinth-package/        # .plnt read/write/validate
    plinth-host-desktop/   # Windows/macOS/Linux entry
    plinth-host-web/
  std/                     # plinth:* .d.ts typings
  examples/
    counter/  todo/  notes/  settings-gallery/
  tests/
    compiler/              # golden tests: .tsx → diagnostics / wasm behavior
    ui/                    # screenshot tests per control and width class
```

`gpui-ce` is a path or git dependency on the user's fork. It is not copied into this repo.

---

## 15. Milestones

Each milestone has exit criteria. Work in this order. Windows is the first platform.

### M0: Protocol and UI runtime spike
- `wit/plinth/` with `ui` and `ui-api.toml` for about 10 controls (`Screen`, `Section`, `Text`, `Heading`, `Button`, `TextField`, `Toggle`, `List`, `Row`, `Empty`).
- `plinth-ui`: an op decoder, an arena, and adapters for these controls on `gpui-ce`, with the `compact` and `wide` shells.
- A Rust guest written **by hand** and compiled to Wasm. It is a counter and a todo list, and it emits ops through `commit`.
- `plinth-runner-wasmtime` and `plinth-host-desktop` run it on Windows.
- **Exit:** the todo app works. Events make a round trip in under 1 ms. A resize changes the shell between compact and wide.

### M1: Compiler v0
- `oxc` parse, the closed-world resolver, and a checker for the v0 syntax (§4.3).
- JSX lowering and signals (§7), and `plinth-rt` with the GC (§5.4).
- The `plinth check`, `plinth build`, and `plinth dev` commands (desktop).
- **Exit:** `examples/counter` and `examples/todo`, written in Plinth TS, behave the same as the M0 hand-written guests. The artifacts meet the §5.5 size targets.
- **Status (2026-10-05):** done. `plinth-rt` is 59.9 KiB and the counter artifact is 60 KiB, before `wasm-opt`. The e2e tests check the behavior and the sizes. These items move to later milestones:
  - Hot reload restarts the app. It does not keep the signal state yet.
  - `int`, `Map` and `Set` (§4.2) are not supported yet.
  - `plinth check --watch` and `plinth dev` poll the files; they do not use OS file events.

### M2: Full UI API 1.0 and host APIs
- All controls in §6.3, the layout rules in §6.4, the theme in §6.5, and accessibility.
- The `store`, `time`, `clipboard`, `dialog`, and `net` host APIs, with the policy and the consent UI.
- **Exit:** `examples/notes` (a list-detail app with sync) works. `plinth shoot` makes screenshots at all width classes.
- **Status (2026-10-05):** in progress.
  - Done: the UI API 1.2 controls, stack navigation and screen actions (§6.3); `plinth:time`, `plinth:store` and `plinth:clipboard` with the policy (§8.5, §11); `examples/settings-gallery`, `examples/contacts` and `examples/timer`; `examples/notes` keeps its notes in the kv store; `plinth-shoot` (§13).
  - Not done: accessibility, the consent UI (a declared capability is granted), `dialog` and `net`, `DatePicker`, `Image` and `Canvas`, and arguments for `navigate.push`.

### M3: Compiler v1
- Classes, unions, generics, `async`/`await`, and the tsserver plugin.
- **Exit:** the compiler test suite covers every rejected-feature diagnostic with a golden test.

### M4: Packaging and desktop targets
- `.plnt` read, write, and validate, and single-file export for Windows, Linux, and macOS.
- **Exit:** one `.plnt` runs the same on all three desktop OSes. The single-file exports start with a double-click.

### M5: Web host
- `plinth-runner-web`, `jco` glue, and the `gpui_web` host.
- **Exit:** the same `.plnt` runs in Chrome, Firefox, and Safari. The single-file `app.html` works without threads.

- **Status (2026-10-05): early web host.** `web/` loads an unchanged `.plnt` and the same core file that the desktop uses. It instantiates the core, then the app module with the core's exports, `memory`, `table` (the core exports it with `--export-table`) and `table_base`, without a link step (the side-module model of §18.3). A JavaScript port of the protocol and a DOM renderer show all UI API 1.2 controls, the primary-screen navigation, stack navigation with a back button, and screen actions. Timers fire `timer` events. `store.kv` uses `localStorage` with the key prefix `plinth:<app id>:`, and only when the manifest declares the capability. `clipboard` keeps a cached value, because the browser Clipboard API is asynchronous. Node tests run the counter, todo, timer and notes packages. Not done: the width classes of §6.1 (CSS media queries stand in for them), icons and images, accessibility checks, the selected state of the segmented `Picker`, dark styles for native inputs, and gpui-ce's web backend.

### M6: Mobile hosts
- This milestone depends on Q1. It adds the AOT path for iOS and wasmtime for Android.
- **Exit:** `examples/notes` runs on a physical device of each platform.

### M7: Hub
- Publishing, signing, the review pipeline, and the client in the runtime.

### M8: Plinth browser and progressive loading
- The browse mode, the app cache, and chunked artifacts (§18).
- **Exit:** a user opens a hub app from a link, uses it, and the host evicts it later. A large app shows its first screen before all of its chunks load.

---

## 16. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| The compiler scope (type checker + codegen) grows too large | High | Keep the v0 subset small. Use golden tests. Reject features early and add them later. |
| Structural typing costs | Medium | Use itables only at interface boundaries. Concrete types use direct offsets. Design this in M1, not later. |
| The bugs in a linear-memory GC | Medium | GC only when idle (no stack roots). Use stress modes: collect after every event in tests. |
| `gpui-ce` has no mobile backends in the checked fork | High for M6 | Find the mobile fork (Q1). Keep all mobile-specific code behind the runner and host API adapters. |
| The semantic UI is too limited for some apps | Medium | `<Canvas>` escape hatch. Grow the UI API through minor versions, driven by real apps. |
| No JIT on iOS changes distribution | Medium | The AOT build is part of the iOS packaging. Decide the hub model for iOS early (Q3). |
| Two instances of `gpui_web` and app Wasm on the web, and COOP/COEP | Low/Medium | Single-thread mode by default for the single-file export. |

---

## 17. Open questions

| # | Question | Notes |
|---|---|---|
| Q1 | Where is the `gpui-ce` fork with the iOS and Android backends? | `C:\repos\gpui-ce` has only Windows, macOS, Linux, and web. |
| Q2 | Final project name | "Plinth" is a placeholder. |
| Q3 | How the hub distributes iOS apps | Per-app IPAs, or a container app with AOT-compiled apps built in. |
| Q4 | Should the runtime embed `gpui-component`, or own all controls? | Evaluate in M0. |
| Q5 | String indexing semantics | UTF-16 compatibility vs. code points. Pick before M1 ends. |
| Q6 | Guest-side or host-side list virtualization | Host-side is better for scroll speed. Guest-side is simpler. |
| Q7 | Is the artifact a component or a core module plus a custom ABI? | **Decided:** the package holds a core app module that imports `plinth-rt` through a versioned app ABI (§10.4). The host links it into its runtime and runs a `plinth:app` component (WIT) on desktop. |
| Q8 | Multi-window apps | Not in UI API 1.0. Add with `ui.window.multi` later. |
| Q9 | How do chunks share memory and the runtime? | Core modules that import one memory, one table and `plinth-rt` from the entry module, or a component-model feature. It is related to Q7. **Partly decided:** an app module already imports the memory, the table and the runtime (§10.4); a lazy chunk can use the same form. |
| Q10 | The cache policy for browsed apps | The size budget, the time limit, the eviction order, and what happens to the data of an evicted app (§18.2). |
| Q11 | Consent for browsed apps | Grants for one session or persistent grants. Which capabilities a browsed app can get without an install (§18.2). |
| Q12 | The address of a browsed app | A hub id, a `plinth://` URL, or an `https://` URL to a `.plnt`. What is the origin of an app: the publisher key or the URL? |

---

## 18. Future: browse mode and progressive loading

> Status: design notes. M8 implements them. The items in §18.4 have an effect on the earlier milestones.

### 18.1 Two usage modes

1. **Installed.** The user installs the apps that they like on a phone or a desktop. Installed apps stay on the device, work offline, and get updates.
2. **Browsed.** The user opens an app from a link or from the hub, as with a web page. The host loads the app on demand and runs it. The host keeps it in a cache for some time, and then removes it. The user can install a browsed app at any time ("keep this app").

Thus the desktop host and the web host are also a **Plinth browser**. The browser has an address input or a hub view, an app switcher, and a package cache.

### 18.2 App cache

- The cache is a content-addressed store. The key of each package and each chunk is its digest.
- Each app has one of two states:
  - `cached`: the host can remove it. Eviction uses a size budget, a time limit, and least-recently-used order.
  - `installed`: the host never removes it automatically.
- Installing a cached app changes only its state. The host does not download it again.
- The compiled-code cache (§9.2) uses the same digests.
- The data and the capability grants of a browsed app have open policy questions (Q10, Q11).

### 18.3 Progressive loading (chunks)

A large app must not be one monolithic module. A user must see the first screen before the full app loads.

- **Split points.** The compiler splits an app at screen boundaries. A screen can declare a lazy component, for example `component: () => import("./reports")`. The import stays inside the closed world (§4.1).
- **Chunks.** The compiler makes one entry chunk (the app shell, the first screen, and `plinth-rt`), one chunk for each split point, and shared chunks for code that two or more chunks use.
- **Linking.** All chunks of one app share one linear memory, one GC heap, and one function table. Thus each lazy chunk imports these items from the entry chunk. This is the "side module" model of dynamic linking. The component model does not supply it today (Q9, Q7).
- **Loading.** The guest asks for a chunk through a host call that completes with a `completion` event (§8.4). The host can also prefetch chunks from hints in the manifest, for example the `primary` screens.
- **Package.** The manifest lists each chunk with its digest and size. An installed `.plnt` contains all chunks. In browse mode, the hub serves each chunk as a separate file. The signature covers the digest of each chunk, so the host can verify each chunk alone.

### 18.4 Effects on the current milestones

- The runner API must load from a chunk source (a package, a cache, or the network), not only from one byte buffer. The M0 runner takes one buffer. Change it before M4.
- `plinth-rt` and the codegen must permit a split in which chunks import the memory, the table, and the runtime functions. Decide this in M1 (Q9), before the code size work.
- The package signature (§10.1) uses one digest for each entry, so the host can verify chunks separately.
- A host must be able to run, suspend, and drop more than one app. Each app has its own guest instance and its own UI root. The Wasm engine is shared.

---

## 19. References

- `C:\repos\krate`: the package container (`crates/bundle`), the policy (`crates/policy`), WIT worlds (`wit/krate/phase4/world.wit`), the import checker (`crates/tools/src/bin/check-component-imports.rs`), and the Pulley/iOS notes (`crates/runtime`).
- `C:\repos\gpui-ce`:
  - `crates/gpui/src/element.rs`: `Element`, `Render`, `RenderOnce`
  - `crates/gpui/src/window.rs`: `Window::draw`, `draw_roots`
  - `crates/gpui/src/app.rs`: `Application`, `run`, `run_embedded`, `open_window`
  - `crates/gpui_platform`: backend selection
  - `crates/gpui_web/examples/hello_web`: web entry
- `oxc`: <https://github.com/oxc-project/oxc>
- `wasmtime`, `wit-component`, `wasm-encoder`, `jco`: Bytecode Alliance
- `wasm2c` (WABT), WAMR AOT: iOS AOT options
