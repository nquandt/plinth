# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first, then `SPEC.md` (the design) and `README.md` (the commands).

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package. The package holds **only app code** (a counter app is about 2 KB) and is the **same file for every platform**. A user installs a host one time (the `plinth` CLI or the `plinth-host` runner). The host has one or more **runtime cores** (`plinth-rt` builds, which are Wasm), links the app into the core that it needs, runs it in wasmtime, and renders a semantic, adaptive UI with `gpui-ce` (the user's fork at `../gpui-ce`). Apps declare intent (controls, sections, rows); the runtime owns layout, spacing and color. See `SPEC.md` §1–§3, §10.4, §10.5.

## 2. State (2026-10-05)

| Milestone | State |
|---|---|
| M0, M1 | done |
| M2: UI API 1.2 controls, navigation, host APIs, accessibility | mostly done (see §9) |
| M3: compiler v1 | partly done: unions, generic functions, `int`, `Map`/`Set`, nullable boxing |
| App-only packages and runtime cores (§10.4, §10.5) | done |
| `plinth native` single-file export (§10.3) | done for the current OS |
| CI on GitHub (`nquandt/plinth`, private) | Windows job tested; Linux/macOS jobs are best effort |
| npm publishing | pipeline written (`.github/workflows/release.yml`, `docs/RELEASING.md`); **not published** |

All work is on branch `master` and pushed to `origin` (https://github.com/nquandt/plinth). All tests pass.

Example apps (Plinth TS, all valid TypeScript): `counter`, `todo`, `notes` (kv storage), `calculator`, `settings-gallery` (the input controls), `contacts` (list-detail, stack navigation, Sheet, Tabs, destructive action), `timer` (`setInterval`).

## 3. Build, run, test

Prerequisites: Rust stable, `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Windows (the only tested host platform). Node 22 for `tsc` and the npm scripts.

```sh
cargo build -p plinth-cli -p plinth-shoot            # target/debug/plinth.exe, plinth-host.exe, plinth-shoot.exe
plinth dev examples/todo                              # window + hot reload (keeps module-level signals)
plinth new C:/path/my-app
plinth check|build <dir>                              # build writes <dir>/dist/<name>.plnt (app code only)
plinth run <file.plnt>                                # or: plinth-host <file.plnt>
plinth native <file.plnt | dir> [-o app.exe]          # one executable: plinth-host + the .plnt payload
plinth core list | install <core.wasm> | export <file>
plinth validate <file.plnt>
plinth-shoot <file.plnt> <out-dir>                    # headless PNGs of every screen at compact/regular/wide

cargo test -p plinth-compiler      # e2e, frontend, lang, hostapi, regions, split, hotreload, link_spike
cargo test -p plinth-rt -p plinth-protocol -p plinth-ui -p plinth-package -p plinth-runner-wasmtime -p plinth-host-desktop
cargo test -p plinth-shoot --test a11y                # headless AccessKit check
node scripts/npm-pack.mjs                             # npm tarballs (after a release build)
```

Run the test crates **one at a time** (§7). The examples are valid TypeScript: `cd examples/<x> && npx -y -p typescript@7 tsc -p .` prints nothing.

## 4. Repository map

| Path | What |
|---|---|
| `SPEC.md` | The design, in ASD-STE100 Simplified Technical English; keep that style. §10.4/§10.5 packages and cores, §13 tooling, §15 milestone status, §18 browse mode. |
| `wit/plinth/app.wit` | Guest/host contract: `ui`, `dev`, `error`, `time`, `store`, `clipboard`; exports `init`, `on-event`. |
| `wit/plinth/ui-api.toml` | Single source of truth for control, prop, event and enum ids (UI API 1.2). Never renumber ids. |
| `std/*.d.ts` | `plinth:*` typings: `ui`, `core`, `time`, `store`, `clipboard`, `lib`. The test `std_typings_match` keeps them equal to the compiler. |
| `crates/plinth-protocol` | Op buffer and event encoding (`no_std` without `std`). |
| `crates/plinth-rt` | The runtime core (Rust → wasm32, `no_std`): GC, allocator, strings, arrays, signals, UI ops, regions, keyed lists, host API wrappers (`host.rs`), hot-reload snapshot (`dev` feature). Its custom section `plinth-core` holds the core version. |
| `crates/plinth-link` | Everything a host needs to run a `.plnt`: embeds the built-in cores (`build.rs` builds release and dev blobs), `rt_abi.rs` (the app ABI, append-only), `link.rs` (append linker, stubs unreachable functions), `split.rs` (encode and load app modules), `cores.rs` (installed cores, selection, install). No compiler. |
| `crates/plinth-compiler` | Parser, checker, lowering, codegen. Re-exports `plinth-link`'s modules as `link`, `split`, `cores`, `rt_abi`. |
| `crates/plinth-ui` | Semantic tree (`tree.rs`, navigation stacks, unit-tested), adaptive shell and controls on gpui-ce with AccessKit roles (`render.rs`), theme tokens. |
| `crates/plinth-runner-wasmtime` | Loads components, checks imports, time limit, memory cap, capability `Policy`, kv store, timers, clipboard trait. |
| `crates/plinth-host-desktop` | Host library and the `plinth-host` runner (it also runs a single-file payload). |
| `crates/plinth-package` | `.plnt` zip, manifest, digest, safety checks; single-file payload footer. |
| `crates/plinth-cli` | The `plinth` binary. Templates in `templates/`. |
| `crates/plinth-shoot` | Headless screenshots (needs gpui `test-support`; not a default member). |
| `npm/`, `scripts/`, `.github/`, `docs/RELEASING.md` | npm packages for 4 platforms, pack and version scripts, CI and release workflows. |

## 5. Compiler and package pipeline

`plinth_compiler::compile_ex(fs, capabilities, dev)`:

1. `driver.rs` → `parse.rs` (oxc) → `check/` (types, narrowing, unions, generics by monomorphization, JSX against `controls.rs`, capability checks `PL1007`, reactivity lint `PL2020` in component bodies) → `lower.rs` (signals, effects, JSX, hot-reload keys in dev) → `codegen.rs`.
2. Codegen writes against `split::app_layout`: runtime functions are imports, table indices are relative to the imported global `table_base`.
3. `split::encode_app` writes `app.wasm` (a core module that imports only what it uses, plus the `plinth-core` section with `MAJOR.MINOR`).
4. The compiler then links it like a host does (`split::load_app` + `link::link` + `componentize`), so each build tests the load path. `Artifact { app, runtime, component, core_size, accent }`: the package gets `app`; `plinth dev` and the tests run `component`.

A host: `cores::link_app(app)` picks the installed core (same major, highest minor ≥ the app's), checks every import against that core's exports, remaps indices, and links.

## 6. Decisions to keep

- **A `.plnt` holds only app code and is universal.** Never put runtime code in it, and never tie it to one runtime build. (The user stated this firmly.)
- **Cores are versioned like nvm** (SPEC §10.5). Inside a major version a core only adds functions and constants; each addition increments `CORE_MINOR` in `rt_abi.rs` and the version in `plinth-rt/src/lib.rs` (a test checks they agree). Breaking changes need a new major version.
- **Size policy:** app artifact size counts (counter core module ≤ 60 KiB, component ≤ 80 KiB, app module ≤ 4 KiB in tests). The raw core may grow (256 KiB guard). Keep new runtime features in separate functions; the linker stubs what an app does not reach. Still no `core::fmt`, `format!`, `HashMap` or Unicode tables in `plinth-rt`.
- Linear memory and a Rust runtime, not Wasm GC (SPEC §5.4). The GC runs only between events.
- Components run one time. A signal read in a component body outside JSX/`computed` is not reactive (`PL2020` warns).
- New UI features are intent props in UI API minor versions, never pixel or color props.
- Denied host calls never trap: they return null/no-op and set `lastError()`.
- `plinth native` is platform-specific and contains a host and a core; the `.plnt` stays universal.

## 7. Gotchas

- **Disk:** gpui + wasmtime builds are large. Do not delete things outside this repo without asking.
- **Windows linker errors LNK1318/LNK1201:** run `cargo test -p <crate>` one crate at a time; `CARGO_BUILD_JOBS` 3–6.
- **Nested cargo:** `plinth-link/build.rs` builds `plinth-rt` into `target/rt` and `target/rt-dev`. `PLINTH_RT_WASM` / `PLINTH_RT_WASM_DEV` use prebuilt files.
- **Worktrees for parallel agents:** make them as siblings (`C:\repos\plinth-wt-<name>`) so `../gpui-ce` resolves, and copy `target/debug` and `target/rt` into them first (robocopy) to reuse the build cache.
- **Python edits on Windows:** `encoding="utf-8"`, `newline="\n"`. Put long scripts in files (very long bash heredocs fail to parse).
- **GUI checks:** prefer `plinth-shoot` (headless). For live clicks, post `WM_MOUSEMOVE` then the click with client-relative coordinates, about 0.5 s apart.
- **`PLINTH_CORES_DIR`** overrides the cores directory (tests use it).

## 8. Working conventions (from the user)

- Commit after each tested step; push to `origin master`. No GPG signing; no attribution lines.
- The user may edit `SPEC.md`; diff it before you change it.
- Subagents (Sonnet) work well with small, explicit task lists (one or two items), their own worktree, and the rules in a shared file. With long lists they tend to stop after the first item.

## 9. Next steps (suggested order)

0. **Plinth Hub** (`docs/HUB.md`): the user's direction for the product. Start with phase H0 (local library, grants store, install-time consent screen in the desktop host), then H1 (signing, the shared capability map, reachable-capability analysis).

1. **Web host (M5) early spike:** export the core's table; JS glue that instantiates the core and then `app.wasm` with the core's exports, memory, table and `table_base` (no static link needed in a browser); JS implementations of the core's WIT imports at the core-module level; a minimal DOM renderer for the op buffer, or gpui-ce's web backend.
2. **Hot reload:** keep signals with array and object values (component-local and row signals are kept now).
3. **Host APIs:** consent UI (today a declared capability is granted), `dialog`, `net` (async with `completion` events; needs callbacks or `async`/`await`).
4. **Controls:** `DatePicker`, `Image` (asset tokens), `Canvas`, arguments for `navigate.push`, animated indeterminate `Progress`.
5. **Compiler v1:** generic interfaces, classes, `JSON`, `async`/`await`, `try`/`catch` (SPEC §5.6).
6. **Distribution:** a core registry (download cores from the hub or npm), publish `@plinth/cli*` and `create-plinth`, verify the Linux and macOS builds.
7. **Open questions** in SPEC §17 (Q1 mobile gpui fork, Q3 iOS distribution, Q10–Q12 browse mode).
