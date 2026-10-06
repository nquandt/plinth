# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first, then `SPEC.md` (the design) and `README.md` (the commands).

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a small WebAssembly component (about 60 KiB). A native host runs it in wasmtime and renders a semantic, adaptive UI with `gpui-ce` (the user's fork at `../gpui-ce`). Apps declare intent (controls, sections, rows); the runtime owns layout, spacing and color. See `SPEC.md` §1–§3.

## 2. State (2026-10-05)

| Milestone | State |
|---|---|
| M0: protocol, UI runtime, wasmtime runner, desktop host | done |
| M1: compiler v0, `plinth` CLI, `.plnt` packages, size targets | done |
| UI API 1.1: `Group`, `Button.size`, `Text`/`Heading` `align` | done |
| npm distribution | scaffolded and tested locally; **not published** |
| M2: full UI API 1.0 controls, host APIs (store, net, …), consent UI | **next** |

All work is committed on branch `master` (9 commits, newest `e34816e`). The working tree is clean. All tests pass.

Four example apps in Plinth TS work end to end: `examples/counter`, `examples/todo`, `examples/notes`, `examples/calculator`.

## 3. Build, run, test

Prerequisites: Rust stable (1.97 used), `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Windows (the only host platform tested). Node 22 for the npm tests and `tsc`.

```sh
cargo build --release -p plinth-cli                  # target/release/plinth.exe (~44 MB; compiler + runtime + dev host)
./target/release/plinth.exe dev examples/todo        # window + hot reload
./target/release/plinth.exe new C:/path/my-app       # new project
./target/release/plinth.exe check|build <dir>        # build writes <dir>/dist/<name>.plnt
./target/release/plinth.exe run <file.plnt>

cargo test -p plinth-compiler                        # front end, link, e2e of all 4 examples, size targets
cargo test -p plinth-rt -p plinth-protocol -p plinth-ui -p plinth-package
node scripts/npm-pack.mjs                            # npm tarballs in target/npm (after a release build)
```

Run the test crates **one at a time** (see §7, linker and disk).

The examples are valid TypeScript too: `cd examples/todo && npx -y -p typescript@7 tsc -p .` must print nothing.

## 4. Repository map

| Path | What |
|---|---|
| `SPEC.md` | The design. It is written in ASD-STE100 Simplified Technical English; keep that style when you edit it. §18 is the future browse mode and chunked loading. §15 has milestone status. |
| `wit/plinth/app.wit` | The guest/host contract: `ui.commit(ops)`, `dev.log(msg)`, exports `init`, `on-event`. |
| `wit/plinth/ui-api.toml` | **Single source of truth** for control, prop, event and enum ids (UI API 1.1). `plinth-protocol/build.rs` generates Rust constants from it. Never renumber ids. |
| `std/*.d.ts` | The `plinth:*` typings for the editor (`lib.d.ts` replaces the ES lib; `ui.d.ts`, `core.d.ts`). |
| `crates/plinth-protocol` | Op buffer and event encoding. `no_std` without the default `std` feature. |
| `crates/plinth-rt` | Guest runtime library, Rust → wasm32, **`no_std`**: GC (mark-sweep, only when idle), allocator, strings (JS number formatting via `ryu`), arrays, signals/computed/effects with owner scopes, UI ops, regions, two-way bindings, keyed list reconciler. Exports the `__plinth_rt_*` ABI. |
| `crates/plinth-compiler` | The compiler. See §5. `build.rs` builds `plinth-rt` for wasm32 (nested cargo, `target/rt`) and embeds it. |
| `crates/plinth-ui` | Host side: semantic tree arena (`tree.rs`, gpui-free, unit-tested), adaptive shell and controls on gpui-ce (`render.rs`), theme tokens (`theme.rs`). |
| `crates/plinth-runner-wasmtime` | Loads components, checks imports (only `plinth:app/*`), time limit (epochs) and memory cap. |
| `crates/plinth-host-desktop` | Host library (`run(HostApp, reload_rx)`) and the `plinth-host` binary. |
| `crates/plinth-package` | `.plnt` zip: manifest, digest, safety checks on read. |
| `crates/plinth-cli` | The `plinth` binary: `new`, `dev` (poll-based watch + hot reload), `check [--json] [--watch]`, `build`, `run`, `validate`. Templates in `templates/`. |
| `npm/` | `@plinth/cli` (JS shim that runs the platform binary), `@plinth/cli-win32-x64`, `create-plinth`. |
| `crates/plinth-guest`, `examples/*-rs` | M0 hand-written Rust guests. Kept for host tests (`plinth example <crate>` builds them). |

## 5. Compiler pipeline

`plinth_compiler::compile(&dyn FileSystem)`:

1. `driver.rs`: load `app/main.tsx` and its imports. Closed world: only `./relative` (inside `app/`) and `plinth:ui` / `plinth:core`. Dependency order; cycles are errors.
2. `parse.rs`: oxc 0.153 → own AST (`ast.rs`). Unsupported syntax is rejected here with stable codes (`diag.rs`, `PL1xxx`–`PL4xxx`). Diagnostics print in tsc format, or JSON.
3. `check/`: type checker → typed IR (`tir.rs`). Contextual typing for callbacks, null narrowing (`if`, `&&`, ternary, early return), destructuring, JSX prop checking against `controls.rs`, `app()` config. All conversions are explicit `Coerce` nodes. Std signatures are built in (`check/stdlib.rs`); the test `std_typings_match` keeps them equal to `std/*.d.ts`.
4. `lower.rs`: signals, computed, effects and JSX → runtime calls. A prop whose expression contains a call becomes an effect. Expression children become regions. `List` → keyed reconciler. Makes the `main` function.
5. `codegen.rs`: Wasm with `wasm-encoder`. Every function takes an env first. Captured variables live in heap frames (per call, and per loop iteration for loop-body variables). Thunks adapt closures to the runtime's `(env) -> ()` callables.
6. `link.rs`: the **append linker** adds the app after `plinth-rt` (no index in the runtime changes), grows the table, sets the start function, removes `__plinth_rt_*` exports, then `wit-component` makes the component. `rt_abi.rs` must match `plinth-rt/src/lib.rs`; the linker checks names and types.

## 6. Decisions to keep

- Linear memory and a Rust runtime, not Wasm GC (SPEC §5.4, because of iOS AOT).
- The GC runs only between events, so there are no stack roots.
- Components run one time. A signal read outside JSX or `computed` is not reactive. (The notes app had this bug; the fix moved the read into the JSX.)
- `plinth-rt` must stay **≤ 60 KiB** and the counter artifact **≤ 80 KiB**. The e2e test checks this. The runtime is at 59.9 KiB: there is very little margin. Do not add `core::fmt`, `format!`, float formatting from `core`, `HashMap`, or Unicode tables to `plinth-rt`. Run `twiggy top` on a non-stripped build to find growth (`CARGO_PROFILE_WASM_STRIP=false`).
- New UI features are intent props (UI API minor versions), never pixel or color props.
- The `plinth` binary is self-contained (no Rust toolchain for authors). The npm name `plinth` is taken; the packages use `@plinth/cli` and `create-plinth` (SPEC Q2).
- The documented deviations from JS are in SPEC §4.6.

## 7. Gotchas

- **Disk space.** gpui + wasmtime debug builds are large. The dev profile uses line tables only and no debug info for dependencies (root `Cargo.toml`). The disk filled up once; the user cleared space. Do not delete things outside this repo without asking.
- **Windows linker errors LNK1318/LNK1201** when many large test binaries link at once: run `cargo test -p <crate>` one crate at a time.
- **Nested cargo:** `plinth-compiler/build.rs` builds `plinth-rt` into `target/rt`. Set `PLINTH_RT_WASM` to use a prebuilt file.
- **`plinth-rt` features:** it uses `plinth-protocol` with `default-features = false` (a path dependency, not `workspace = true`).
- **Python edits on Windows:** open files with `encoding="utf-8"` and write with `newline="\n"`, or `§` and line endings break. `.gitattributes` normalizes to LF.
- **Shell heredocs:** very long bash commands with embedded Python failed to parse once; put long scripts in a file in the scratchpad.
- **GUI checks:** a PowerShell helper that resizes, clicks (posted mouse messages) and captures the window with `PrintWindow` worked well. Pause about 0.5 s between clicks; fast posted clicks hit the wrong keys. `plinth dev` runs as a background task; stop it with `Stop-Process -Name plinth`, which reports the task as "failed" (expected).
- **`npx <tarball>`** runs nothing on Windows; use `npx --package=<tarball> create-plinth <dir>`.

## 8. Working conventions (from the user)

- Commit after each tested step. Do not GPG-sign (`commit.gpgsign=false` is set in the repo). No attribution lines in commit messages.
- The user may edit `SPEC.md` while you work. Diff it (ignore line endings) before you change it.
- Subagents worked well for example apps: give them the release binary, forbid cargo builds and edits outside their folders, and ask for repro snippets of compiler gaps.

## 9. Next steps (suggested order)

1. **M2 controls** (SPEC §6.3): `TextArea`, `Picker`, `Checkbox`, `Slider`, `NumberField`, `Progress`, `Badge`, `Image`/`Icon` set, `Tabs`, `Sheet`, `Dialog`, `Menu`, `Grid`, Screen `actions`. For each: add ids to `ui-api.toml`, typings, `controls.rs`, `render.rs`, and a test. Also stack navigation (`navigate.push`/`back`) and non-primary screens (today they give a warning and do not show).
2. **M2 host APIs:** `store` (kv first), `time`, `clipboard`, `dialog`, `net` as WIT interfaces with the capability policy (SPEC §8.5, §11) and the consent UI. Async calls return a request id and finish with a `completion` event; this needs `async`/`await` (M3) or callbacks first.
3. **Compiler gaps:** `int`, `Map`/`Set`, generics in user code, classes, unions with narrowing, `try`/`catch` (M3 list). A lint for a signal read outside a reactive scope would catch the notes bug class.
4. **Hot reload that keeps signal state** (SPEC §13).
5. **Accessibility:** AccessKit roles for every node (SPEC §9.3).
6. **Publishing:** CI that builds the per-platform binaries and publishes `@plinth/cli*` and `create-plinth`; decide the final name (Q2).
7. **Open questions** in SPEC §17 (Q1 mobile gpui fork, Q3 iOS distribution, Q9 chunk linking for progressive loading).
