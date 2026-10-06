# Plinth

Build cross-platform apps from a strict subset of TypeScript with JSX. The compiler turns each app into a small WebAssembly component (about 60 KiB for a counter). A native host runs it and renders a semantic, adaptive UI with [`gpui-ce`](https://github.com/nquandt/gpui-ce). See [SPEC.md](SPEC.md).

```tsx
import { app, signal, Screen, Section, Text, Button } from "plinth:ui";

function Home() {
  const count = signal(0);
  return (
    <Screen title="Hello">
      <Section title="Counter">
        <Text>{`You pressed the button ${count()} times.`}</Text>
        <Button label="Press me" role="primary" onPress={() => count.update((n) => n + 1)} />
      </Section>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", icon: "house", component: Home } } });
```

## Start an app

```sh
npm create plinth@latest my-app   # when the packages are published
cd my-app && npm install
npm run dev                       # the app reloads when you save
npm run check                     # type-check
npm run build                     # dist/<name>.plnt
```

From a clone of this repo, use the binary directly:

```sh
cargo run --release -p plinth-cli -- new my-app
cargo run --release -p plinth-cli -- dev my-app
```

`plinth` commands: `new`, `dev`, `check [--json] [--watch]`, `build`, `run <app.plnt>`, `validate`.

## Status

| Milestone | State |
|---|---|
| M0: protocol and UI runtime | done |
| M1: compiler v0, `plinth` CLI | done: Plinth TS → Wasm component, hot reload, `.plnt` packages, size targets met |
| M2: full UI API 1.0, host APIs | next |

## Repository

| Part | Where |
|---|---|
| WIT world and UI API id table | `wit/plinth/` |
| `plinth:*` typings | `std/` |
| Compiler: parse (oxc), check, lower JSX and reactivity, codegen, link | `crates/plinth-compiler` |
| Guest runtime (no_std): GC, strings, arrays, signals, ops | `crates/plinth-rt` |
| Op and event encoding | `crates/plinth-protocol` |
| Semantic tree, adaptive shell, controls on gpui-ce | `crates/plinth-ui` |
| wasmtime runner | `crates/plinth-runner-wasmtime` |
| Desktop host (library and `plinth-host`) | `crates/plinth-host-desktop` |
| `.plnt` packages | `crates/plinth-package` |
| `plinth` CLI | `crates/plinth-cli` |
| npm packages (`@plinth/cli`, `create-plinth`) | `npm/`, `scripts/npm-pack.mjs` |
| Examples in Plinth TS | `examples/counter`, `examples/todo` |
| M0 hand-written Rust guests | `crates/plinth-guest`, `examples/counter-rs`, `examples/todo-rs` |

`gpui-ce` must be checked out next to this repo (`../gpui-ce`). The compiler builds `plinth-rt` for `wasm32-unknown-unknown` in its build script, so install the target: `rustup target add wasm32-unknown-unknown`.

## Test

```sh
cargo test -p plinth-compiler      # front end, link, and e2e tests of the examples
cargo test -p plinth-rt -p plinth-protocol -p plinth-ui -p plinth-package
```
