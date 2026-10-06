# Plinth

Plinth is a framework for cross-platform apps. You write a strict subset of
TypeScript with JSX ("Plinth TS"). The compiler turns your app into a small
WebAssembly package, a `.plnt` file. One `.plnt` file runs on every
platform: the same file works on Windows, and later on macOS, Linux, the
web, and mobile. A host (the `plinth` CLI today; the Plinth Hub later) runs
the package and renders a native, adaptive UI.

## Why Plinth

- **No platform SDK.** You do not install Xcode, Android Studio, or
  Electron. You write Plinth TS and run `plinth dev`.
- **One file for every platform.** The package holds only your app code.
  It carries no runtime and no platform-specific code. The host supplies
  the runtime.
- **Capabilities you can verify.** An app can reach the network, storage,
  or the clipboard only through host APIs that it both declares and
  imports. A tool can read the package and prove what it can do; it does
  not have to trust the manifest.
- **Apps are tiny.** A counter app compiles to about 2 KB. A small app
  with several screens is a few hundred KB.
- **You declare intent, not pixels.** Your UI code names controls,
  sections, and roles. The runtime owns layout, spacing, color, and
  accessibility, and adapts the same tree to a phone, a tablet, and a
  desktop window.

## Status

Plinth is pre-1.0 and under active development. **Windows is the only
tested platform today.** The core language, the UI controls, and the
desktop host work and are covered by tests. Read [HANDOFF.md](HANDOFF.md)
for the full, current state.

| Area | State |
|---|---|
| Plinth TS compiler (v0 core language) | works: types, control flow, closures, JSX, generics, classes (single inheritance), unions, `Map`/`Set` |
| UI API 1.3 controls and navigation | works on the desktop host |
| Host APIs: `plinth:time`, `plinth:store`, `plinth:clipboard`, `plinth:dialog` | work on the desktop host |
| `plinth:net` | in progress (needs `async`/`await`) |
| Desktop host (Windows) | works, tested in CI |
| Desktop host (macOS, Linux) | builds; not yet tested |
| Web host | early spike in `web/`; several host APIs are stubs |
| Mobile hosts (Android, iOS) | not started |
| `async`/`await`, `try`/`catch` | not started |
| Plinth Hub (app store, capability consent, publishing) | design only, see [docs/HUB.md](docs/HUB.md) |
| npm packages (`@plinth/cli`, `create-plinth`) | pipeline written, not yet published; build from source |

## 60-second quick start

Today, build the `plinth` tool from this repository (npm packages are not
published yet; see [docs/getting-started.md](docs/getting-started.md) for
full setup, including prerequisites):

```sh
cargo build --release -p plinth-cli
alias plinth=target/release/plinth
```

Then:

```sh
plinth new my-app        # a hello-world app (same as: npm create plinth@latest my-app, once published)
cd my-app
plinth dev                # a window; the app reloads when you save
plinth check               # type-check, no build
plinth build                # writes dist/<name>.plnt
plinth run dist/*.plnt       # run the built package
plinth native dist/*.plnt -o MyApp.exe   # one executable: host + core + app
```

## A small example

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

`count` is a signal: a reactive value. `count()` reads it inside a JSX
slot, so the `Text` updates itself when the count changes. There is no
virtual DOM and no re-render of the whole component.

## Learn more

| Document | Content |
|---|---|
| [docs/getting-started.md](docs/getting-started.md) | Install, your first app, project layout, the dev loop, navigation, saving data, packaging. |
| [docs/language.md](docs/language.md) | Plinth TS: supported syntax, types, what is rejected and why, reactivity rules, the standard library. |
| [docs/ui.md](docs/ui.md) | Every UI control and its props, intent rules, layout, navigation, accessibility. |
| [docs/host-apis.md](docs/host-apis.md) | `plinth:time`, `plinth:store`, `plinth:clipboard`, `plinth:dialog`, capabilities, and the manifest. |
| [docs/architecture.md](docs/architecture.md) | An overview of the pipeline, the crates, and the hosts, for contributors. |
| [SPEC.md](SPEC.md) | The full design specification. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Build and test commands, conventions, where to start. |

## Repository layout

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
| Early web host spike | `web/` |
| npm packages (`@plinth/cli`, `create-plinth`) | `npm/`, `scripts/npm-pack.mjs` |
| Example apps in Plinth TS | `examples/` |

`gpui-ce` must be checked out next to this repo (`../gpui-ce`). The
compiler builds `plinth-rt` for `wasm32-unknown-unknown` in its build
script, so install the target: `rustup target add wasm32-unknown-unknown`.

## License

Apache-2.0 (see `Cargo.toml`). A `LICENSE` file is pending; see
[CONTRIBUTING.md](CONTRIBUTING.md).
