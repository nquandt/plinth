# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first. Then read:

- `SPEC.md`: the design (ASD-STE100 Simplified Technical English; keep the style; status notes are inline, search "Status").
- `CONTRIBUTING.md`: commands.
- `docs/ui.md` (all controls, Level 2 primitives, Canvas, lifecycle), `docs/host-apis.md`, `docs/language.md`.
- `docs/GAPS.md`: known gaps and what was fixed. The newest sections: "Robustness", "Desktop frame cost", "Async code size", "wasmi engine".
- `docs/PRODUCTION.md`: the production readiness checklist (gate levels Preview, Beta, 1.0). Update an item's status when its work merges.
- `docs/UPDATES.md`: updates without a reinstall (design and status).
- `docs/HUB.md` (the App Hub), `docs/REGISTRY.md`, `docs/web-hub.md`.
- Design drafts: `docs/VALIDATION.md`, `docs/STORAGE.md`, `docs/UI-ADVANCED.md`, `docs/EXPERIENCE.md`.
- `infra/self-hosted-runner/README.md`: the CI runners on the owner's PC.

## 0. Read this first

**State (2026-10-08, pause point):** `master` is the active branch. One agent branch is open: `wt/upgrade` (worktree `U:\plinth-wt\upgrade`), D4 `onUpgrade` steps A and B; step C is the first next step (§10). The production checklist `docs/PRODUCTION.md` is the plan to 1.0; `bash scripts/production-status.sh` counts it (Preview 26/26 Done; Beta 25 Done, 14 Partial, 13 Open). The gate passes on `master`: `bash scripts/ci-local.sh full` and `node web/test/run-a11y.mjs`. GitHub has `master` up to `b6e766e`; more than 115 commits after it are local. **Push only when the owner asks**, and only commits that passed the gate. A push starts CI; read §0.2 first.

### 0.1 Owner decisions (keep them)

1. **One implementation for all hosts.** One `.plnt` runs on every host; a host supplies only host APIs and host-owned UI (consent window, error and stop banners, dialogs). Never build a second UI for one host.
2. **Mobile hosts render with gpui.** Only the web host renders to the DOM.
3. **The web host stays DOM** (SPEC M5). A `gpui_web` canvas host is a possible later experiment, not planned.
4. **Every web app runs in a sandboxed iframe** (opaque origin), the web Hub app too; the parent page is the host shell (SPEC §10.3).
5. **Level 2 UI:** typed style props, spacing units (4 px) and theme tokens, no raw pixels or colors.
6. **No size limit on the runtime** (core, `plinth-rt`). App code (`.plnt`) should stay small, as a goal, not a cap.
7. **Data belongs to the user:** "your device or your storage, never the vendor's"; strong walls between apps (`docs/STORAGE.md`).
8. **Plain UI words and safe defaults** (`docs/EXPERIENCE.md` §2, §4).
9. **Balance:** the web host is one part. Prefer the compiler, the desktop/gpui host, the UI, storage and platforms.
10. **Updates without a reinstall (2026-10-07):** three layers: native host, core, app. A native update only when the native interop (a host interface) changes. App authors choose when their app takes a new core. Design: `docs/UPDATES.md`.
11. **iOS runs Wasm with the `wasmi` interpreter (2026-10-07),** no JIT (SPEC §9.2). Pulley (wasmtime's interpreter) and WasmKit were the other candidates; wasmi 2 is about two times faster than Pulley.
12. **Production readiness (2026-10-07, `docs/PRODUCTION.md` §15):** 1.0 covers all platforms, mobile too (assume gpui mobile backends exist). 1.0 needs localization and sound. Support is rolling with no guaranteed window; every released runtime stays available. License: MIT OR Apache-2.0. Name, public repo, Linux and macOS windows: at the end. CI runners and npm: near Beta. Keep `docs/LIMITS.md` and `CHANGELOG.md` current.
13. **App tests are written in Plinth TS (2026-10-08, T3):** `*.test.tsx` files with `plinth:test`, run by `plinth test` and `npm test`. Not a JavaScript package. Design: `docs/TESTING.md`.
14. **Linux and macOS work waits for agents on those machines (2026-10-08).** Until then, track each item in `docs/PLATFORM-TODO.md`, and do not mark such an item Done from Windows.
15. **Orchestrate subagents (2026-10-08):** this session plans, reviews, merges and runs the gate. Clear implementation work goes to Sonnet agents (`model: "sonnet"`), mechanical test work to Haiku (`model: "haiku"`, Haiku 5.5). Split each task small: one layer for each agent (for example WIT + core + desktop; then compiler + tests; then web + docs), each step buildable and testable alone. A Sonnet agent refused a full end-to-end feature as too big for one pass.
16. **The web Hub server and the Cloudflare tunnel are stopped (2026-10-08).** Do not start them again unless the owner asks.

### 0.2 Open items for the owner (remind them)

1. **Finish the self-hosted CI runners** (`infra/self-hosted-runner/README.md`). Done: both agents downloaded, `ci.yml` moved to them (commit `4aff175`), the WSL packages installed, the WSL service `plinth-runner` installed and enabled (not started). Left for the owner: (a) `gh auth token | wsl -d Ubuntu -- bash -c 'umask 077; cat > ~/.plinth-runner-token'`; (b) in an elevated PowerShell, `infra\self-hosted-runner\install-windows-service.ps1`. Then the agent: `sudo systemctl start plinth-runner` in WSL (the owner gave the WSL sudo password in the session of 2026-10-07; ask again if needed), check that both runners are online (`gh api repos/nquandt/plinth/actions/runners`), push when the owner asks, and watch the first CI run. **Until the runners are online, a push queues CI jobs that nothing runs** (the hosted Actions minutes ran out on 2026-10-06).
2. **Remove a scratch folder:** an agent left `U:\plinth-wt\icutry-target` (ICU4X experiments). The safety check of the agent session blocks a recursive delete; the owner runs `rm -rf U:/plinth-wt/icutry-target`. (The old worktrees of 2026-10-07 are already removed.)
3. **Decisions:** UP-Q1..Q6 in `docs/UPDATES.md` (the core policy default is `follow` for now); the project key that signs cores (also the Hub key, HANDOFF of 2026-10-06); `docs/HUB.md` H-Q4; the open questions in the design drafts (ST-Q*, UA-Q*, UX-Q*).
4. **Play Pong again** (`plinth dev examples/pong`, a release build is better) and say if the motion is smooth now (§10 item 4).
5. **Manual checks that only the owner can do:** a screen reader pass (Narrator on Windows; U3, U4) and Japanese or Chinese IME input in a text field (U5).

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package that holds **only app code** (the counter is about 2 KB) and is the **same file on every platform**. A user installs a host one time: the `plinth` CLI, the `plinth-host` runner, or the Plinth Hub. The host has **runtime cores** (`plinth-rt` builds, Wasm, `MAJOR.MINOR.PATCH` plus a digest), links each app into the core that it needs, runs it in wasmtime (desktop), wasmi (planned for iOS) or the browser (web), and renders with `gpui-ce` (the owner's fork at `../gpui-ce`) or the DOM. Level 1 controls declare intent and the runtime owns the look; Level 2 primitives (`Box`, `Span`, `Pressable`, `Scroll`, `Canvas`) give apps their own look from theme tokens. A host proves from an app's imports which capabilities the app can use, and the user approves them.

## 2. State

| Area | State |
|---|---|
| Cores (SPEC §10.4, §10.5) | Core **1.12.0, shipped and frozen on 2026-10-07** (`rt_abi::SHIPPED_MINOR`, `crates/plinth-link/core-abi.snapshot`, the file in `cores/shipped/`). **Core 1.13 is open** (not shipped): `sql_call`, the `kv_*` hash table for `Map`/`Set`, the `locale_*` functions (and, on `wt/upgrade`, `app_data_version`/`app_mark_data_version`). Every 1.13 item is in `docs/migrations/core-1.13.md`; a test requires a guide for each new minor. An app that uses `Map`, `Set`, `sql`, `plinth:locale` or `onUpgrade` declares 1.13; others keep their old minor. Versions are `MAJOR.MINOR.PATCH` plus a SHA-256 digest; author policy `core = "follow" \| "patch" \| "pinned"`. |
| Memory (2026-10-07) | The allocator uses best fit and keeps the rest of a split block; array buffers count toward the next collection. Before, three leaks grew memory without a limit (GAPS "Robustness", monkey test). |
| UI API (`ui-api.toml`, `docs/ui.md`) | **1.13** (not shipped): Level 1 controls, Level 2 primitives, pointer events, `onContextMenu`, a `Menu` that opens at a point, drag and drop (`dragValue`, `onDrop`, `List.onReorder` with Alt+Up/Down), `Dialog` children. A modal layer blocks pointer input on both hosts. The theme follows the OS dark mode, high contrast, text size and reduced motion (U7, Windows and web). |
| Host APIs (`docs/host-apis.md`) | `time` (with `onFrame`), `store` (kv and `sql`), `clipboard`, `dialog`, `notify`, `net`, `files` (private space), `hub` (privileged), `locale` (tag, direction, formatting), `error.report`. Every cell of the "Support on each host" test table has a test (H7). Async calls return a `Promise` or take a `done` callback. Denied calls never trap. |
| Limits and stop reasons | Desktop runner: 2 s per call (epochs), 64 MiB memory (`Limits`); a trap gives a plain one-line reason in "This app stopped" ("array index out of bounds", "out of memory", "too many nested calls (stack overflow)", "the app is not responding (time limit reached)"). The web host has the same limits, banners and reasons: each app runs in a Worker with a 2 s watchdog (`CALL_TIME_LIMIT_MS`) and a 64 MiB memory cap (`MEMORY_LIMIT_PAGES`) (GAPS R1, R2). |
| Engines | wasmtime (default). **wasmi** (`PLINTH_ENGINE=wasmi`, cargo feature `wasmi`): every compiler suite that runs apps passes on it; `bash scripts/ci-local.sh wasmi` and a CI job run it (not the slow `lang`, `sevenguis`, `monkey`, `budgets`). Fuel time limit, `FUEL_PER_MS` = 2,000,000 (release numbers). A release build of wasmi is 7 to 11 times slower than wasmtime; a debug build is 100 to 2,000 times slower (opt-level 3 on `wasmi` crashes a Windows debug build). Open: R5 (a host state crate without wasmtime, for iOS). |
| Compiler (SPEC §4) | Unions and narrowing, generics, classes, `int`, `Map`/`Set` (hash tables), tuples, patterns, JSON, fragments, checked casts, string and array methods (stable merge sort), RegExp, `try`/`catch`/`finally`, `async`/`await`. Long chains (100,000 terms) compile without recursion (L6); real nesting over 1000 levels is `PL1008`. `===` on strings and `T \| null` primitives compares values. Golden tests for every diagnostic. `plinth test` builds a test build (`docs/TESTING.md`). |
| Updates (`docs/UPDATES.md`) | Steps 1–3 done (core patch versions and policy, signed feeds, the desktop updater with rollback), the restart prompt and re-consent. `onUpgrade(fromVersion)`: steps A and B on `wt/upgrade`, step C open (§10). Open: the web service worker cache (D5), the project core key (S6), mobile. |
| Desktop Hub (`docs/HUB.md`) | H0, H1 (Ed25519 signing), H3 steps 1–3. |
| Web Hub (`docs/web-hub.md`) | The same Hub app in a sandboxed frame with a `plinth:hub` bridge; browse mode; consent; digest and Ed25519 checks. |
| Web export (SPEC §10.3) | `plinth build <dir> --target web [--single-file]`; `web/plinth.js` is GENERATED (`node scripts/gen-plinth-js.mjs`; commit it). |
| Storage | `kv`, `files` (private space) and `store.sql` (SQLite on every host; `docs/SQL.md`). `examples/notes` keeps its notes in SQL with FTS5 search. |
| Validation apps | 7GUIs 1–7, Pong, primitives, files-demo, notes, and 20 other examples. `examples/todo` and `examples/counter` have `plinth test` tests. |
| Platforms | Windows: everything. Linux and macOS: compiler, cores, runner, hub and registry tests pass; no GUI window yet; all their work is listed in `docs/PLATFORM-TODO.md`. Mobile: not started (decided: gpui + wasmi on iOS). |
| CI | `ci.yml` runs Windows and Linux on self-hosted runners (not online yet, §0.2), macOS weekly on GitHub. |

## 3. Build, run, test

Prerequisites: Rust stable, `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Node 22, Edge (browser tests). Windows is the main platform.

```sh
bash scripts/ci-local.sh full            # THE gate (5-10 min with a warm cache)
bash scripts/ci-local.sh full --shoot    # plus the headless GPU tests (screenshots, frame cost, hostile render)
node web/test/run-a11y.mjs               # headless Edge: axe, renderer checks, banners, web export, web Hub
node web/test/run-a11y.mjs --typing-only # renderer checks only; --hub-only; --export-only
node scripts/gen-plinth-js.mjs           # regenerate web/plinth.js after a change to a bundled web file
node scripts/gen-web-ids.mjs             # regenerate web/ui-api.js after a change to ui-api.toml
bash scripts/ci-local.sh wasmi           # the app suites on the wasmi engine
plinth test examples/todo                # app tests (docs/TESTING.md)
bash scripts/production-status.sh [--open Beta]   # checklist counts and open items

plinth dev examples/pong ; plinth build <dir> [--target web] ; plinth run <app.plnt> ; plinth native <app> -o app.exe
plinth core list|install|export ; plinth hub … ; plinth registry build|serve …
bash scripts/web-hub-demo.sh             # web Hub with the examples on http://127.0.0.1:8787/
plinth-shoot <app.plnt> <out-dir>        # headless screenshots at three widths
```

Stability tests (all in the gate unless marked):

| Test | What | Knobs |
|---|---|---|
| `plinth-compiler/tests/robustness.rs` | Mutated example sources never panic the compiler; deep nesting; changed app modules never panic the linker | `PLINTH_FUZZ_ITERS`, `PLINTH_FUZZ_SEED`; a stack overflow writes the case to `target/fuzz-failures/current.txt` |
| `plinth-compiler/tests/monkey.rs` | Random UI events on every example, normal and GC stress: no trap, valid tree, level memory | `PLINTH_MONKEY_STEPS`, `PLINTH_MONKEY_APP`, `PLINTH_MONKEY_TRACE` |
| `plinth-compiler/tests/errors.rs` "Limits" | Endless loop, recursion, allocation stop only the app | — |
| `plinth-ui/tests/hostile_ops.rs` | Random op streams keep the tree valid | `PLINTH_FUZZ_ITERS` |
| `plinth-shoot/tests/hostile_render.rs` | Random trees with bad props draw with no panic (`--shoot`) | `PLINTH_FUZZ_ITERS` |
| `plinth-shoot/tests/frame_cost.rs` | Frame cost of every example and of each form control (`--shoot`) | `PLINTH_FRAME_APP` |
| `plinth-compiler/tests/async_size.rs` | Code size of `await` against callbacks (`--ignored`) | — |
| `web/test/run-stop.mjs` | Web host stopped state and reasons | — |

**The owner's machine:** `plinth` and `plinthw` are installed in `%USERPROFILE%\.cargo\bin` (`cargo install --path crates/plinth-cli --locked --force`; reinstall after CLI changes; it fails with "Access is denied" while a `plinth.exe` from there runs). A personal publisher key "Nate" exists; `PLINTH_HUB_TRUSTED_KEYS` holds its id. The web demo uses a throwaway key in `target/web-hub-demo-key`; a demo server can run from a copy, `target/demo-bin/plinth-demo.exe`, so that it does not lock `target/debug/plinth.exe`. Start a Cloudflare tunnel (`cloudflared tunnel --url http://localhost:8787`) only when the owner asks; its URL is public. `C:` has little free space (about 59 GB); `U:` has 2.2 TB.

Test-only env vars: `PLINTH_CORES_DIR`, `PLINTH_HUB_DIR`, `PLINTH_PUBLISHER_DIR`, `PLINTH_HUB_TRUSTED_KEYS`, `PLINTH_BLESS=1` (goldens), `PLINTH_TRACE_RENDER=1` (frame times), `PLINTH_ENGINE=wasmi`.

## 4. Repository map

| Path | What |
|---|---|
| `wit/plinth/app.wit` | Guest/host interfaces: `ui`, `dev`, `error`, `time`, `store`, `clipboard`, `dialog`, `notify`, `net`, `hub`, `files`, `sql`, `locale`. Append only (`app.wit.snapshot`). |
| `docs/migrations/` | One guide for each core minor (`core-1.13.md`). |
| `docs/PLATFORM-TODO.md` | The work for Linux and macOS machines. |
| `wit/plinth/ui-api.toml` | Control, prop, event and enum ids (UI API 1.13). **Append only.** |
| `std/*.d.ts` | Typings of the `plinth:*` modules; `std_typings_match` keeps them equal to the compiler (`check/stdlib.rs`). |
| `crates/plinth-rt` | The core (Rust → wasm32, `no_std`): GC (`gc.rs`), allocator (`allocator.rs`), reactivity, UI ops, JSON, strings. |
| `crates/plinth-link` | Embedded cores, `rt_abi.rs` (append-only, `ADDED_IN`, `CORE_MINOR`, `CORE_PATCH`), `link.rs`, `split.rs`, `cores.rs` (versions, digest, selection by policy). |
| `crates/plinth-compiler` | Parser (oxc; `parse.rs`, `MAX_DEPTH`), checker (`check/`), lowering, codegen. `controls.rs` is the control and prop table. Tests in `tests/`. |
| `crates/plinth-ui` | Semantic tree (`tree.rs`), gpui renderer (`render.rs`), Level 2 (`primitives.rs`), `canvas.rs`, theme, charts. |
| `crates/plinth-runner-wasmtime` | Runner and `Guest` (an enum of engines), `HostState` (all host logic, shared by both engines), `wasmi_core.rs`/`wasmi_engine.rs` (feature `wasmi`), `Policy`, kv, timers, net, files, hub backend trait. |
| `crates/plinth-host-desktop` | Host library and `plinth-host`, consent window, core selection by the manifest's policy. |
| `crates/plinth-hub`, `plinth-registry`, `plinth-package`, `plinth-cli` | Library and grants; registries and the static server (embeds `web/`); `.plnt`, manifests (`core`, `core-digest`) and signatures; the CLI. |
| `crates/plinth-shoot` | Headless screenshots and GPU tests. |
| `web/` | Browser host (`plinth-web.js`, `dom-renderer.js`, `protocol.js`, …), the frame pair, the Hub shell, generated `plinth.js` and `ui-api.js`; tests in `web/test/` (fixtures in `web/test/fixtures/`). |
| `infra/self-hosted-runner/` | CI runner scripts and docs. |

## 5. How a new control or prop goes in (the usual path)

1. Append ids to `ui-api.toml` (bump `version` and the assert in `plinth-protocol`'s test); `node scripts/gen-web-ids.mjs`.
2. `crates/plinth-compiler/src/controls.rs` (the `box_props!` macro for Level 2 boxes); a new `PropTy` needs a case in `check/jsx.rs` (optional props go through `typed_prop`). Typings in `std/ui.d.ts`; names in `check/stdlib.rs` `UI_NAMES`.
3. Desktop: `render.rs` dispatch arm and `primitives.rs` for style props. Keep layout flat: each nested flex box around a subtree about doubles its layout work (GAPS F1); use block layout where flex gives nothing, and check with `frame_cost.rs`.
4. Web: `dom-renderer.js` (`makeView`, `primitiveStyle`), then `node scripts/gen-plinth-js.mjs`.
5. Tests on both hosts; goldens for new diagnostics; look at the screenshots.

## 6. Rules to keep

- A `.plnt` holds only app code and is universal.
- Cores: inside a major version only add functions; each addition increments `CORE_MINOR` (and resets `CORE_PATCH`), gets an `ADDED_IN` entry, and needs every host (desktop runner, both engines, AND `web/plinth-web.js`) to supply new WIT functions. A fix with no new function bumps `CORE_PATCH`. Prefer generated code over new runtime functions. 1.12 is shipped and frozen (2026-10-07): the next new function goes into 1.13 (`CORE_MINOR = 13`, `ADDED_IN` 13); `tests/core_abi_frozen.rs` enforces it. A new WIT item also needs a blessed `wit/plinth/app.wit.snapshot`.
- No runtime size limit; keep `plinth-rt` free of `core::fmt`, `format!`, `HashMap` and Unicode tables anyway. The counter's `app.wasm` is guarded at 8 KiB (`tests/e2e.rs`).
- New UI: intent props or typed Level 2 props from tokens; never raw pixels or colors. `ui-api.toml` ids are append-only.
- Denied host calls never trap. `hub.manage` only for packages signed by a trusted Hub key. Host path checks for `files` on both hosts.
- The runtime skips unchanged props and text, except `value`.
- The GC runs only between events. Anything the core allocates for app data must count toward the next collection (`allocated_since_gc`).
- The compiler must never panic or overflow its stack on any input: add a regression case to `robustness.rs` for each crash found.

## 7. Gotchas

- Windows linker: test crates run one at a time (the script does it; `CARGO_BUILD_JOBS` 3–6).
- `plinth-link/build.rs` builds `plinth-rt` into `target/rt` and `target/rt-dev`. After a runtime change, **export the core again** (`plinth core export target/core.wasm`) before Node or Edge tests. `ci-local.sh full` does it.
- `registry serve --web` serves the web files embedded in the binary: rebuild the CLI before a web Hub test.
- Goldens change when a message or a list of names changes; re-bless with `PLINTH_BLESS=1 cargo test -p plinth-compiler --test golden` and read the diff.
- **Editing files from the shell:** Python on Windows writes CRLF unless a file is opened with `newline=''` (the repo uses LF). Backslashes in heredocs and in `sed`/PowerShell arguments get lost or turn into control characters (`\r` became a CR in a path, `C:\repos` became `C:epos`). For Windows paths and escapes, use the Edit tool or a script file in the scratchpad.
- The Bash hook blocks raw `grep`/`find`: use `rg` or the Grep tool.
- **The agent session's safety check** blocks moving credentials (a GitHub token into WSL) and deleting folders recursively (old worktrees). Give the owner the command instead; do not work around it.
- **Never run a long gate in the background while you edit sources:** later steps then compile the new, untested code. Stop it, stash, or wait.
- Headless Edge: use the DevTools protocol; it cannot shrink below about 500 px; it sometimes drops the first click into a new iframe; `run-a11y.mjs` has a 60 s limit per DevTools call. A killed run leaves Edge helper processes. The primitives check timed out at page load one time in three runs; run it again. The web App Hub check sometimes times out at the Hub start in a full run ("Failed to fetch" or `dataset.ready` never set; about one run in four on 2026-10-07); `--hub-only` and the next full run pass. Find the cause when there is time (a slow first fetch from `registry serve` after the export checks?).
- AccessKit bounds in `plinth-shoot` tests are device pixels (scale 2 headless): divide by `window.scale_factor()`.
- Live GUI clicks from an agent session often do not reach the window; prefer headless tests.
- **Timings are noisy while other builds run** (agents). Compare variants interleaved in one process (switch draw by draw), not in separate runs.
- gpui-ce: `taffy.rs` `compute_layout` computes a subtree again when an element asks for layout as a root, and the Taffy tree is built again each frame (GAPS F1). Agents in `U:\plinth-wt\` build from the same gpui-ce checkout through a junction: do not change gpui-ce while agents run.

## 8. Working conventions (from the owner)

- Commit after each tested step. No GPG signing; no attribution lines.
- The owner may edit `SPEC.md`; diff it before you change it.
- **Subagents** (§0.1 decision 15): worktrees `U:\plinth-wt\<name>` (`git worktree add U:/plinth-wt/<name> -b wt/<name> master`; `U:\plinth-wt\gpui-ce` is a junction, so `../gpui-ce` resolves). Run one build agent at a time. Brief each agent with the design, the files to read and the exact test commands; tell it to work only in its worktree, to run only the targeted tests of the crates it changes (never `ci-local.sh`), to commit on its branch, never to push, and not to edit `HANDOFF.md` or `SPEC.md` (it reports what SPEC needs; the orchestrator edits SPEC). Then review the diff, merge into `master`, run `ci-local.sh full` (and `run-a11y.mjs` for UI or web changes), update SPEC, and remove the worktree. If the gate stops early, the example packages can be stale: fix the gate, then run `run-a11y.mjs` again.
- Write all prose in ASD-STE100 Simplified Technical English.

## 9. Session log 2026-10-07 (what changed and why)

- **Robustness:** compiler mutation test (fixed a `break` outside a loop, JSON of recursive types, deep nesting with `stacker` and `PL1000`); runner limit tests; plain stop reasons on both hosts; web host stopped state and banners; hostile op streams, hostile render trees and changed app modules (no bugs found; tests kept).
- **Language:** default values in patterns, object rest, optional props set to `undefined` for every prop kind, `onMount`.
- **Memory:** three core leaks found by the monkey test and fixed (allocator tail loss, first fit, uncounted array buffers).
- **Performance:** frame-cost survey; labelled form controls about two times faster (block layout; GAPS F1); async code size measured.
- **Platform decisions:** `docs/UPDATES.md` (three-layer updates), wasmi for iOS, core patch versions and the `core` policy.
- **CI:** self-hosted runner files and `ci.yml` (owner steps open, §0.2). Pushed to GitHub up to `b6e766e`.

## 9a. Session log 2026-10-07, second part (production checklist)

- **`docs/PRODUCTION.md`:** gates Preview, Beta, 1.0 and Later; owner decisions in §15 (1.0 = all platforms, localization and sound; rolling support; MIT OR Apache-2.0; no sync in 1.0). Preview is complete. New docs: `docs/LIMITS.md`, `CHANGELOG.md`, `SECURITY.md`, `docs/THREAT-MODEL.md`, `docs/COMPATIBILITY.md`, `docs/PRIVACY.md`, `docs/I18N.md` (design), `docs/DEPENDENCIES.md`, `docs/guides/`, `CODE_OF_CONDUCT.md`.
- **Security fixes:** `net.local` covered too few private ranges (both hosts; the desktop now also checks the address after DNS resolution); the package signature did not cover `manifest.toml` (now `plinth.signature/2`, which refuses `/1`).
- **Built:** RegExp (linear-time Pike VM in the core; literals, `new RegExp`, `test`, `exec`; collectable objects); notify step 1 (WIT, core, runner; no compiler module yet); updater re-consent and the restart prompt; package format compatibility; a WIT append-only snapshot test (`wit/plinth/app.wit.snapshot`, bless with `PLINTH_BLESS=1`); the web 64 MiB memory cap and a Worker for each web app; `cargo deny` and `npm audit` in the gate; desktop keyboard and accessibility for every control (`plinth-shoot/tests/a11y_controls.rs`).
- **Flaky tests fixed:** six test files named temp folders from the clock only (100 ns steps on Windows); they now add an atomic counter.
- **Lesson (agents):** seven parallel agents with cold worktree builds overloaded the PC for hours. Run at most one or two build agents; tell an agent to run only the tests of the crates it changes, then merge into `master` and run the full gate there (the cache of `master` is warm). Give an agent one bounded step; a large task makes it stop with no code.

## 9b. Session log 2026-10-08 (store.sql, core 1.13, UI, test kit)

- **Core 1.13 is open** (1.12 is frozen). `docs/migrations/` (X5) lists every item; `plinth-link/tests/migration_guides.rs` requires a guide for each new minor.
- **`store.sql` (H4, Done):** SQLite on the desktop (`rusqlite`, `plinth-runner-wasmtime/src/sql.rs`) and the web (the official SQLite Wasm build, vendored in `web/vendor/sqlite-wasm/`, in a Worker of the page with OPFS); the same authorizer, limits and error texts on both (`tests/sql_cases.txt`); folder web exports; `examples/notes` with FTS5. The desktop database is in `<app id>+sql/`, out of reach of `plinth:files`.
- **wasmi (R4, Q4, Done):** all app suites pass; `FUEL_PER_MS` from release numbers; a gate mode and a CI job.
- **Speed and memory:** `Map`/`Set` hash tables (about 120 times faster), merge sort (about 110 times faster), `JSON.stringify` and `join` without quadratic memory.
- **Locale (L5, Done):** `locale()`, `direction()`, `formatNumber`, `formatDate`, `pluralCategory`, `formatList`, `compare` (ICU4X on the desktop, `Intl` on the web); lifecycle kind 4 on a locale change. 8 of 448 cases differ between the hosts, each with a reason (`docs/I18N.md` §3.7).
- **Compiler:** long chains without recursion and `PL1008` (L6, Done); `===` on strings and nullable primitives compares values, and a signal set with an equal value does nothing.
- **UI:** `onContextMenu`, `Menu` at a point, a modal layer blocks the pointer, drag and drop, `Dialog` children (U6, Done); OS dark mode, high contrast, text size and reduced motion (U7, Done on Windows and the web).
- **Test kit (T3 step 1):** `plinth:test` and `plinth test`; no core or WIT change (a private protocol over `dev.log`).
- **Host API tests (H7, Done):** `clipboard` on three hosts, `dialog` on the web.
- **Docs:** `docs/PLATFORM-TODO.md`, `docs/TESTING.md`, `docs/migrations/`; SPEC §4.6, §4.7, §5.1, §6.3, §6.5, §7.1, §8.1, §8.4, §8.5, §9, §10.5, §13 updated.
- **Lessons:** a stopped gate leaves stale example packages, so `run-a11y.mjs` then fails for the wrong reason; a Sonnet agent needs a task of one layer; `run-a11y.mjs` checks must wait until a control is enabled, because each app runs in a Worker.

## 10. Next steps (in order)

`bash scripts/production-status.sh --open Beta` lists the open Beta items. Work that needs Linux or macOS is in `docs/PLATFORM-TODO.md`.

1. **D4 `onUpgrade`, step C** (branch `wt/upgrade`: step A `f332df8` = WIT `app-info`, core `app_data_version`/`app_mark_data_version`, desktop storage in `<data dir>/apps/<id>/data-version`; step B `e0d616a` = the host rule, `plinth:ui` `onUpgrade`, `Guest::set_app_version` before `init` and `Guest::finish_start` after it, `tests/upgrade.rs` 8/8 on both engines; the web host still has stub imports): the web host (store the data version with the web kv keys; `data-version` returns a version only when an upgrade is pending; store the version after start unless the app claimed the upgrade), a Node test, the docs (`docs/UPDATES.md` §6, §9.1 and the UP-Q5 interim rule, `docs/migrations/core-1.13.md`, `docs/host-apis.md`, PRODUCTION D4, CHANGELOG), then SPEC (§4.7 `onUpgrade`, §8.5). Merge, run the gate and `run-a11y.mjs`.
2. **T3 step 2:** the same app tests on wasmi and on the web host; the missing steps (dialogs, network answers, pointer and key steps, Slider, DatePicker, back navigation); `beforeEach`, `describe`, `only`, `skip` (`docs/TESTING.md` §12).
3. **Small compiler gaps:** a non-async callback that mixes `return;` and `return <promise>` gives an invalid Wasm module (GAPS, found 2026-10-08; fix this first, a compiler must never emit an invalid module); `===` on a union of `string` and an object type (GAPS); a `??`/`?.` chain of about 50,000 links needs too many Wasm locals (GAPS R3).
4. **C6:** freeze the 1.0 language subset in `docs/language.md`.
5. **D5:** web updates with a service worker cache (UPDATES §9 step 4).
6. **T4:** an inspector for the UI tree, signals and host calls. **T6:** `plinth publish` to a registry.
7. **V2 and V3:** the flagship notes app (rich text is the largest design question, VALIDATION §3) and an app suite in one host.
8. **R7 / GAPS F1:** make nested flex layout linear in gpui-ce (only when no agent builds), then the gpui-ce one-line fix for text input names (`EditableTextElement::write_a11y_info`, U3), and pass `WM_SETTINGCHANGE` to the app (U7; then the one-second poll can go).
9. **H3:** notifications, the rest (Linux and macOS are in PLATFORM-TODO).
10. **Owner items:** the CI runners (§0.2), signing (P5), the release key (S6), npm publishing (D1), manual screen reader and IME checks (U3, U4, U5).
11. **Later (1.0):** platforms (PLATFORM-TODO), mobile with gpui and wasmi (R5, P7, P8), sound (G7), the docs website (X3).
