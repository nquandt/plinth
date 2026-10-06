# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first. Then read `SPEC.md` (the design), `docs/HUB.md` (the app hub), `docs/REGISTRY.md` (registries), and `CONTRIBUTING.md` (commands). `docs/GAPS.md` lists the known language gaps.

## 0. Read this first: the push rule

**Never push before `bash scripts/ci-local.sh full` passes locally.** CI runs the same script. The owner's GitHub Actions minutes are nearly used up (about 90 % of the month on 2026-10-06), and earlier pushes failed CI because only parts of the tests ran locally. Batch commits into few pushes, and ask the owner before a push.

**Update (2026-10-06, later): the Actions minutes are fully used up.** Do not push. Keep commits local on `master`. Until the minutes reset or a self-hosted runner exists, the local `ci-local.sh full` pass is the only gate.

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package that holds **only app code** (a counter app is about 2 KB) and is the **same file on every platform**. A user installs a host one time: the `plinth` CLI, the `plinth-host` runner, or later the Plinth Hub app. The host has **runtime cores** (`plinth-rt` builds, which are Wasm and versioned like nvm), links each app into the core that it needs, runs it in wasmtime, and renders a semantic, adaptive UI with `gpui-ce` (the owner's fork at `../gpui-ce`). Apps declare intent; the runtime owns layout, spacing and color. A host can prove from an app's imports which capabilities the app can use, and the user approves them (the Hub's main idea, `docs/HUB.md` §1.1).

## 2. State (2026-10-06)

| Area | State |
|---|---|
| Packages and cores (SPEC §10.4, §10.5) | Done. Core version is **1.10** (1.9: the Hub calls of H3 step 3; 1.10: `uncaught`, `set_drain`, `report` for exceptions and `async`, branch `wt/lang5`). An app declares the lowest core that has the functions it imports. |
| UI API (SPEC §6.3, `docs/ui.md`) | **1.5**: about 30 controls incl. Checkbox, TextArea, Slider, NumberField, Picker, Progress, Badge, Tabs, Sheet, Dialog, Menu (anchored popover), Grid, Action, Image (package assets), Icon, DatePicker, Chart (bar/line/pie), Row `trailing`. Stack navigation, screen actions, AccessKit roles, keyboard use. |
| Lists | Keyed reconciler with a minimal-moves diff; host-side virtualization above 200 rows (variable row heights). 10,000 rows: about 4 ms per frame. |
| Host APIs (SPEC §8.5, `docs/host-apis.md`) | `time` (timers, dates, time zone), `store` (kv), `clipboard`, `dialog` (async), `net` (HTTP fetch, per-host capabilities), `hub` (privileged). Async calls use request ids and `completion` events; without a `done` callback they return a `Promise`. Denied calls never trap. Uncaught app errors go to the host through `error.report`. |
| Compiler (SPEC §4) | Unions and narrowing (also on member paths), generic functions/interfaces/type aliases, classes with single inheritance and `instanceof`, `int`, `Map`/`Set` (iteration, `forEach`, `keys`/`values`/`entries`), fixed-length tuples, nullable boxing, `JSON.stringify`/`JSON.parse<T>`, fragments, `.map()` JSX children, checked `as` casts, many string and array methods (incl. stable `sort`, `splice`, `fill`, `flat`), dynamic-length `Chart` data, `try`/`catch`/`finally`/`throw` (generated exception path, built-in `Error`, uncaught errors reported and the app keeps running), `async`/`await` (continuation-passing transform in `check/asyncfn.rs`, promises and microtask queue as app code). Golden tests for every diagnostic code, and for the new `PL2009`/`PL2010`/`PL3001` messages. |
| Hub (`docs/HUB.md`) | H0 (library, grants, blocks, groups, consent window, risk-level defaults, re-consent on update, global policy), H1 (Ed25519 publisher signing), H3 step 1 (several apps per host, privileged `plinth:hub` for packages signed by a trusted key), H3 step 2 (the Hub UI app `examples/hub`, core 1.8 hub calls incl. async `search`/`install`, the host poll loop that opens launches in new windows, in-process consent, `plinth hub ui|open|shortcut|register-scheme`, `plinth://` links on Windows), H3 step 3 (core 1.9: `appInfo`, `pin`, publisher blocks, async `checkUpdates`/`update`; the Hub UI checks for updates on start, shows a count in the Library and "Update available" on the App screen, updates with re-consent, pins versions, blocks publishers; `.lnk` shortcuts with the app icon and Start menu entries; the `plinthw` launcher with no console window). |
| Registry (`docs/REGISTRY.md`) | Draft 1: static or dynamic registries, `plinth registry build|serve`, `plinth hub source|search|install|update`. |
| Tooling | Hot reload that keeps signal state, headless screenshots (`plinth-shoot`), GC stress mode, `plinth native` single-file executables, VS Code extension (`editors/vscode`, not published), npm packages (smoke-tested locally, not published). |
| Web host (`web/`) | Runs the same `.plnt` and core file in a browser: all UI API 1.5 controls, timers, kv, clipboard, dialogs, net. axe-core: 0 violations on 5 apps. |
| CI | `.github/workflows/ci.yml` runs `scripts/ci-local.sh` on Windows (full) and Linux (no GUI); macOS weekly or by hand. |
| Platforms | Windows: everything. Linux and macOS: compiler, cores, runner, hub and registry tests pass; no GUI window has been opened there yet. Mobile: not started. |

Example apps (all valid TypeScript, all checked by CI): `counter`, `todo`, `notes`, `calculator`, `settings-gallery`, `contacts`, `timer`, `gallery`, `dialogs`, `budget` (data-driven, charts), `utility` (desktop helper), `quotes` (net), `big-list`, `gc-torture`, `hub` (the Hub UI; privileged, see its README).

## 3. Build, run, test

Prerequisites: Rust stable, `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Node 22. Windows is the main platform.

```sh
bash scripts/ci-local.sh full            # THE check before any push (2–3 min with a warm cache)
bash scripts/ci-local.sh full --shoot    # also headless screenshot tests (needs a GPU)

cargo build -p plinth-cli -p plinth-shoot
plinth new C:/path/my-app ; plinth dev <dir> ; plinth check|build <dir> ; plinth run <app.plnt>
plinth native <app.plnt|dir> -o app.exe
plinth validate <app.plnt>               # signature, declared vs reachable capabilities
plinth core list|install|export
plinth publisher init|show ; plinth sign <app.plnt> ; plinth build --sign
plinth hub add|list|run|ui|open|remove|grants|block|unblock|block-publisher|policy|groups|source|search|install|update|pin|shortcut|register-scheme
plinth registry build <folder> [--with-core] ; plinth registry serve <folder>
plinth-shoot <app.plnt> <out-dir>        # PNGs of every screen at compact/regular/wide
node web/test/run-a11y.mjs               # axe-core in headless Edge (local only; needs the CDN)
```

Test crates must run **one at a time** on Windows (linker errors LNK1318/LNK1201 otherwise); the script does this. Test-only env vars: `PLINTH_CORES_DIR`, `PLINTH_HUB_DIR`, `PLINTH_PUBLISHER_DIR`, `PLINTH_HUB_TRUSTED_KEYS`, `PLINTH_BLESS=1` (rewrite diagnostic goldens), `PLINTH_TRACE_RENDER=1` (frame times).

## 4. Repository map

| Path | What |
|---|---|
| `SPEC.md` | The design, in ASD-STE100 Simplified Technical English (keep the style). Status notes are inline (search "Status"). |
| `docs/` | `HUB.md`, `REGISTRY.md`, `GAPS.md`, `language.md`, `ui.md`, `host-apis.md`, `getting-started.md`, `architecture.md`, `RELEASING.md`. |
| `wit/plinth/app.wit` | Guest/host interfaces: `ui`, `dev`, `error`, `time`, `store`, `clipboard`, `dialog`, `net`, `hub`. |
| `wit/plinth/ui-api.toml` | Single source of truth for control, prop, event and enum ids (UI API 1.5). Append only; never renumber. `node scripts/gen-web-ids.mjs` regenerates `web/ui-api.js`. |
| `std/*.d.ts` | Typings for `plinth:ui`, `core`, `time`, `store`, `clipboard`, `dialog`, `net`, `hub`, and `lib.d.ts`. `std_typings_match` keeps them equal to the compiler. |
| `crates/plinth-rt` | The core (Rust → wasm32, `no_std`). Its `CORE_VERSION` section must equal `CORE_MINOR`. |
| `crates/plinth-link` | Embedded cores, `rt_abi.rs` (the app ABI, append-only, `ADDED_IN` per function), `link.rs`, `split.rs`, `cores.rs`, `capabilities.rs` (capability map, risk levels). No compiler. |
| `crates/plinth-compiler` | Parser (oxc), checker, lowering, codegen. Tests in `tests/`: lang, e2e, golden, apps, hostapi, net, async, async_await, errors, gc_stress, hotreload, split, capabilities, list_diff, perf (ignored), grid, regions. |
| `crates/plinth-ui` | Semantic tree, renderer on gpui-ce, theme, calendar math. |
| `crates/plinth-runner-wasmtime` | Runner, `Policy` (incl. `check_net`), kv, timers, request queue, net worker, hub backend trait. |
| `crates/plinth-host-desktop` | Host library (`open_app`, `run`, `run_from_hub`, consent window) and the `plinth-host` runner. |
| `crates/plinth-hub` | Library, grants, blocks, groups, sources, `HubService`. |
| `crates/plinth-registry` | Registry documents, source client, static generator, static server. |
| `crates/plinth-package` | `.plnt` zip, manifest, signatures, publisher keys, single-file payloads. |
| `crates/plinth-cli` | The `plinth` binary and the `plinth new` template. |
| `crates/plinth-shoot` | Headless screenshots and GPU tests (not a default member). |
| `web/` | Browser host (core + app as side modules, DOM renderer); node tests in `web/test/`. |
| `editors/vscode/` | VS Code extension (diagnostics from `plinth check --json`). |
| `npm/`, `scripts/` | npm packages for 4 platforms, pack and version scripts, `ci-local.sh`. |

## 5. Pipeline in one paragraph

`compile_ex(fs, capabilities, dev)`: parse → check (types, narrowing, JSX against `controls.rs`, capability checks `PL1007`, lints) → lower (signals, effects, JSX, hot-reload keys in dev) → codegen against `split::app_layout` (runtime functions are imports; table indices are relative to `table_base`) → `split::encode_app` writes `app.wasm` with the lowest needed core version → the compiler links it as a host would (`load_app` + `link` + `componentize`), so every build tests the load path. A host picks a core (`cores::link_app`), checks the imports against that core's exports, checks that reachable capabilities ⊆ declared ones, verifies the signature, and links.

## 6. Decisions to keep

- A `.plnt` holds only app code and is universal. Never tie it to one runtime build.
- Cores: inside a major version only add functions; each addition increments `CORE_MINOR`, gets an `ADDED_IN` entry, and needs every host (the desktop runner AND `web/plinth-web.js`) to supply any new WIT function. When parallel branches add functions, the coordinator renumbers the minors at merge.
- App artifact size counts, not raw core size; new runtime features go in separate functions (the linker stubs what an app does not reach). No `core::fmt`, `format!`, `HashMap` or Unicode tables in `plinth-rt`.
- New UI is intent props; never pixels or colors. Ids in `ui-api.toml` are append-only.
- Denied host calls never trap.
- `hub.manage` is granted only to packages signed by a trusted Hub key.
- Linear memory and a Rust runtime; the GC runs only between events.

## 7. Gotchas

- Windows linker: one test crate at a time (`CARGO_BUILD_JOBS` 3–6).
- `plinth-link/build.rs` builds `plinth-rt` into `target/rt` and `target/rt-dev`.
- Generated `.plinth/` typings are ignored by git; do not commit them.
- Diagnostic goldens (`tests/golden/`) change when a message changes (for example a new control in the "the controls are: …" help). Re-bless with `PLINTH_BLESS=1 cargo test -p plinth-compiler --test golden` and read the diff.
- Long bash heredocs with Python and backslashes break; put scripts in files (the scratchpad).
- Headless Chromium `--virtual-time-budget` stalls on real Wasm work; `web/test/run-a11y.mjs` uses the DevTools protocol instead.
- Headless Edge cannot shrink below about 500 px; check "compact" at 560 px.
- Live GUI clicks from an agent session often do not reach the window; prefer headless tests (`crates/plinth-shoot/tests/*.rs` show how to drive a headless window through AccessKit bounds).

## 8. Working conventions (from the owner)

- Validate locally before every push (§0). Commit after each tested step. No GPG signing; no attribution lines.
- The owner may edit `SPEC.md`; diff it before you change it.
- Parallel agents: use sibling worktrees (`C:\repos\plinth-wt-<name>`, so `../gpui-ce` resolves), copy `target/debug` and `target/rt` into a new worktree to reuse the build cache, give each agent one or two items, and require `bash scripts/ci-local.sh full` before it reports. Agents never push. Merge their branches locally, run the script, then push in one batch with the owner's approval.

## 9. Next steps (suggested order)

1. **Push the local commits** (§0) when CI minutes are available again and the owner agrees. CI run `37517664495` (commit `38004ef`) was still running at handoff; check it first.
2. **Open decisions for the owner:** the license (Cargo.toml says Apache-2.0; there is no LICENSE file yet; "MIT OR Apache-2.0" is the Rust norm), and whether to use a self-hosted CI runner.
3. **Hub, after H3 step 3:** a Plinth project Hub key (the owner's decision; so the Hub UI needs no `PLINTH_HUB_TRUSTED_KEYS`), first-use prompts (H5; the design needs an answer to `docs/HUB.md` H-Q4 first, and a synchronous call such as `clipboard.readText` cannot wait for a prompt), `addShortcut` from the Hub UI, automatic or daily update checks, and a live GUI check of the Hub UI by a person (the tests are headless; a `.lnk` shortcut was opened by hand once and started the app through `plinthw` with no console window).
4. **Compiler:** `Promise.all`; async class methods; `await` inside `switch`, `do…while` and `try`/`finally` (now `PL2009`); a smaller `async` code size (one shared `then`/`resolve`/`reject` set instead of one per `Promise<T>`); a host UI for `error.report` (the desktop host only logs it now); `Chart` `series` as a non-literal expression, tuples with optional or rest elements, `flat(depth > 1)`. Note: `tests/e2e.rs` now limits the *linked runtime* of the counter (core module less app code) to 60 KiB, the SPEC §5.5 wording; master was already at 61366 B of the old whole-module limit of 61440 B.
5. **Platforms:** open a GUI window on Linux and macOS and fix what breaks; then mobile (SPEC Q1, Q3).
6. **Distribution:** publish the npm packages and a first registry; Hub H2 (signed indexes, transparency log); key rotation.
7. ~~**Charts:** real path-based line and pie geometry on the desktop~~ done (`crates/plinth-ui/src/chart.rs`, test `plinth-shoot/tests/chart_geometry.rs`).
