# Architecture overview

This page is for contributors. It gives a short map of how Plinth works
end to end, and where the code lives. For the full design, read
[SPEC.md](../SPEC.md); for the hub, read [HUB.md](HUB.md).

## The pipeline, in one picture

```
  your Plinth TS source
          |
          v
  +----------------+     +------------------------------+
  |  plinth-compiler |-->| app.wasm (a core Wasm module, |
  |  parse / check /  |   | app code only, a few KB)      |
  |  lower / codegen   |   +------------------------------+
  +----------------+             |
                                  v
                          +--------------+
                          | .plnt package |  (zip: manifest + app.wasm + assets)
                          +--------------+
                                  |
                                  v
                    +--------------------------+
                    |  a host picks a runtime   |
                    |  core and LINKS app.wasm  |
                    |  into it (append linker)   |
                    +--------------------------+
                                  |
                                  v
                       +----------------------+
                       |  wasmtime / the web /  |
                       |  AOT runs the linked   |
                       |  component              |
                       +----------------------+
                                  |
                   op buffer (ui ops)  |  events (ui, completion, timer...)
                                  v
                       +----------------------+
                       |  plinth-ui renders a   |
                       |  semantic, adaptive UI  |
                       |  (gpui-ce on desktop)    |
                       +----------------------+
```

The app never talks to the OS, the network, or the DOM directly. It
only emits an **op buffer** (create/insert/set-prop/listen/...) that
describes a semantic tree, and it only calls host APIs that it both
imports and declares as capabilities. The host owns rendering, layout,
and every side effect.

## App modules and runtime cores

This split (SPEC.md §10.4–§10.5) is the core idea that makes a `.plnt`
universal:

- **`app.wasm`** (inside the `.plnt`) is just your app's code. It has
  no own memory, no own table, and no exports. It *imports* runtime
  functions by name and type from a module called `plinth-rt`. It
  carries a custom section naming the lowest **core version**
  (`MAJOR.MINOR`) that has every function it needs.
- **A core** is one build of `plinth-rt` — the GC, strings, arrays,
  signals, the op-buffer writer, host API wrappers. It is itself a Wasm
  module, so a given core version is byte-identical on every host
  platform. Inside one major version, a core only ever *adds*
  functions; it never removes or changes one. This means the newest
  1.x core can run every app that ever declared a 1.y core
  (`y <= x`).
- **Linking** happens on the host, not at build time: the host picks an
  installed core (same major version, lowest minor version that covers
  the app's needs), checks every import by name and type, remaps
  indices, and appends the app's code after the core's — an *append
  linker*, so no index inside the core ever shifts. The result is one
  linked module that the host actually runs. `plinth-cli` runs this
  same link step after every build, so each build tests the load path
  your users' hosts will take.

Because of this split, the same `.plnt` file — a few KB — runs on
Windows today and, unmodified, on macOS, Linux, the web, and mobile
once their hosts exist, as long as each host carries (or can fetch) a
compatible core.

## Crates (this repository)

| Crate | Role |
|---|---|
| `plinth-compiler` | Parser (built on `oxc`), type checker, JSX/reactivity lowering, codegen, and the append linker. Re-exports `plinth-link`'s linking logic. |
| `plinth-link` | Everything a host needs to run a `.plnt`: the embedded built-in cores, the app ABI (`rt_abi`), the linker, package splitting/encoding, installed-core management. |
| `plinth-rt` | The runtime core itself (`no_std`, compiled to `wasm32-unknown-unknown`): GC, allocator, strings, arrays, signals, UI ops, host API wrappers. |
| `plinth-protocol` | The op buffer and event wire format, shared by the guest runtime and every host. |
| `plinth-ui` | The semantic tree, navigation stacks, the adaptive shell, and the controls, rendered on `gpui-ce`, with AccessKit roles. |
| `plinth-runner-wasmtime` | Loads a linked component in wasmtime: import checking, time limits, a memory cap, the capability `Policy`, the kv store, timers, clipboard. |
| `plinth-host-desktop` | The desktop host library and the `plinth-host` runner binary (Windows today; macOS/Linux build but are untested). |
| `plinth-package` | `.plnt` zip read/write, the manifest, digests, zip-safety checks, and the single-file-export footer format. |
| `plinth-hub` | The local library, grants, and blocks stores behind `plinth hub` (phase H0 of the Hub design; see below). |
| `plinth-cli` | The `plinth` binary that ties all of the above together. |

## Hosts

- **Desktop (`plinth-host-desktop`)** — the only tested host. It uses
  wasmtime to run the linked component and `gpui-ce` to render
  `plinth-ui`'s semantic tree as a native, adaptive window. It is also
  how `plinth dev`, `plinth run`, and `plinth native` work.
- **Web (`web/`)** — an early spike, not a production host. Plain ES
  modules load a core and an app module and link them side-module style
  in the browser, with no bundler. A hand-written `dom-renderer.js`
  draws plain HTML. Several host APIs (`store`, `clipboard`, timer
  events into the app) are stubs today — see `web/README.md`.
- **Mobile (Android, iOS)** — not started. The design (SPEC.md §9.5,
  §11) expects a `gpui-ce` mobile fork, and iOS specifically needs an
  ahead-of-time Wasm translation path, because iOS forbids a JIT.

## The Hub, in brief

The **Plinth Hub** (full design in [HUB.md](HUB.md)) is the planned
end-user app store: it finds apps, keeps a library, shows what each app
can do before running it (computed from its Wasm imports, not just
trusted from its manifest), and supplies runtime cores on demand. It is
itself a privileged Plinth app on top of an ordinary host, plus a small
set of Hub-only host services (library, grants, signing, a source
protocol). Phase H0 (a local library and grants store, driven by
`plinth hub` today) already exists in `plinth-hub`; phases H1 onward
(signing, sources, the Hub UI itself, a hosted registry) are design
only.

## Where to go next

- The full design: [SPEC.md](../SPEC.md).
- The Hub design: [HUB.md](HUB.md).
- Current state and next steps for contributors: [HANDOFF.md](../HANDOFF.md).
