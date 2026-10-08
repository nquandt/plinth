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

**State (2026-10-07, late):** `master` is the only active branch (agent branches `wt/*` are merged or reset; old ones can be deleted). The production checklist `docs/PRODUCTION.md` is the plan to 1.0; `bash scripts/production-status.sh` counts it. The gate passes on it: `bash scripts/ci-local.sh full --shoot` and `node web/test/run-a11y.mjs`. GitHub has `master` up to `b6e766e`; the commits after it are local. **Push only when the owner asks**, and only commits that passed the gate. A push starts CI; read §0.2 first.

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

### 0.2 Open items for the owner (remind them)

1. **Finish the self-hosted CI runners** (`infra/self-hosted-runner/README.md`). Done: both agents downloaded, `ci.yml` moved to them (commit `4aff175`), the WSL packages installed, the WSL service `plinth-runner` installed and enabled (not started). Left for the owner: (a) `gh auth token | wsl -d Ubuntu -- bash -c 'umask 077; cat > ~/.plinth-runner-token'`; (b) in an elevated PowerShell, `infra\self-hosted-runner\install-windows-service.ps1`. Then the agent: `sudo systemctl start plinth-runner` in WSL (the owner gave the WSL sudo password in the session of 2026-10-07; ask again if needed), check that both runners are online (`gh api repos/nquandt/plinth/actions/runners`), push when the owner asks, and watch the first CI run. **Until the runners are online, a push queues CI jobs that nothing runs** (the hosted Actions minutes ran out on 2026-10-06).
2. **Delete the 8 old, merged worktrees** `C:\repos\plinth-wt-{a11y,ci,files,hostapi,inputs,lang,rtfix,structure}` (all clean, all commits in `master`). The safety check of the agent session blocks the delete; the owner runs it:
   ```powershell
   cd C:\repos\plinth
   'a11y','ci','files','hostapi','inputs','lang','rtfix','structure' | ForEach-Object { git worktree remove --force "C:\repos\plinth-wt-$_" }
   git branch -D wt/dx wt/ci wt-files wt/dates wt/golden wt/lang6 wt/web-sandbox wt/web-dom
   git worktree prune
   ```
3. **Decisions:** UP-Q1..Q6 in `docs/UPDATES.md` (the core policy default is `follow` for now); the project key that signs cores (also the Hub key, HANDOFF of 2026-10-06); `docs/HUB.md` H-Q4; the open questions in the design drafts (ST-Q*, UA-Q*, UX-Q*).
4. **Play Pong again** (`plinth dev examples/pong`, a release build is better) and say if the motion is smooth now (§10 item 4).

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package that holds **only app code** (the counter is about 2 KB) and is the **same file on every platform**. A user installs a host one time: the `plinth` CLI, the `plinth-host` runner, or the Plinth Hub. The host has **runtime cores** (`plinth-rt` builds, Wasm, `MAJOR.MINOR.PATCH` plus a digest), links each app into the core that it needs, runs it in wasmtime (desktop), wasmi (planned for iOS) or the browser (web), and renders with `gpui-ce` (the owner's fork at `../gpui-ce`) or the DOM. Level 1 controls declare intent and the runtime owns the look; Level 2 primitives (`Box`, `Span`, `Pressable`, `Scroll`, `Canvas`) give apps their own look from theme tokens. A host proves from an app's imports which capabilities the app can use, and the user approves them.

## 2. State

| Area | State |
|---|---|
| Cores (SPEC §10.4, §10.5) | Core **1.12.0** (1.12 was never shipped, so later additions went into it). Versions are `MAJOR.MINOR.PATCH` (`rt_abi::CORE_PATCH`, bumped by hand for a fix that adds no function) plus a SHA-256 digest; an app still declares only `MAJOR.MINOR`. Author policy in `plinth.toml`: `core = "follow" \| "patch" \| "pinned"` (`cores::link_app_with_policy`; the web host supports only `follow`). 1.12 has `onFrame`, `Math.random`/`seedRandom`, `onCleanup`, `isActive`, `onMount`. The runtime skips unchanged prop and text ops (G8). |
| Memory (2026-10-07) | The allocator uses best fit and keeps the rest of a split block; array buffers count toward the next collection. Before, three leaks grew memory without a limit (GAPS "Robustness", monkey test). |
| UI API (`ui-api.toml`, `docs/ui.md`) | **1.13**: Level 1 controls, Level 2 primitives with partial styles, absolute positions, keys, pointer events, Canvas with stroke shapes. An optional prop of any kind takes `T \| null` (`icon={c ? "check" : undefined}`): `null` removes the prop on both host trees. |
| Host APIs (`docs/host-apis.md`) | `time` (with `onFrame`), `store` (kv), `clipboard`, `dialog`, `net`, `files` (private space), `hub` (privileged), `error.report`. Async calls return a `Promise` or take a `done` callback. Denied calls never trap. |
| Limits and stop reasons | Desktop runner: 2 s per call (epochs), 64 MiB memory (`Limits`); a trap gives a plain one-line reason in "This app stopped" ("array index out of bounds", "out of memory", "too many nested calls (stack overflow)", "the app is not responding (time limit reached)"). The web host has the same banners and reasons (`PlinthApp.stopped`), but no time limit or memory cap of its own (GAPS R1, R2). |
| Engines | wasmtime (default). **wasmi** (`PLINTH_ENGINE=wasmi`, cargo feature `wasmi` on `plinth-runner-wasmtime`; `wasmi_core.rs`, `wasmi_engine.rs`): steps 1–2 merged. The core module is taken out of the component; all 41 host imports call the same `HostState` methods as wasmtime; the time limit is fuel (`FUEL_PER_MS` = 150,000, set before each call); the memory cap is the same `Limiter`. On wasmi: errors and limits 28/28, language 171/171, 7GUIs 7/7, apps 7/7 (debug build; language takes about 12 min because the inner wasmi crates are optimized but `wasmi` itself is not — opt-level 3 on `wasmi` crashes a Windows debug build, GAPS "wasmi engine"). What is left: §10 item 1. |
| Compiler (SPEC §4) | Unions and narrowing, generics, classes, `int`, `Map`/`Set`, tuples, rest patterns in array and object destructuring, default values in patterns, JSON (also recursive types), fragments, `.map()` children, checked casts, string and array methods, `try`/`catch`/`finally`, `async`/`await`, the Promise API. A nesting limit of 1000 levels (`PL1000`); each compile runs on a 256 MiB stack (`stacker`). Golden tests for every diagnostic. |
| Updates (`docs/UPDATES.md`) | Steps 1–3 done: core patch versions and the `core` policy; a signed feed (`feed.json` + detached Ed25519 `feed.json.sig`, `plinth-package::feed`), `plinth feed add`, `[updates]` in `plinth.toml`; the desktop updater for `plinth native` (`plinth-host-desktop::updater`: a background check, key continuity with the embedded package, no downgrade, digests, rollback to the embedded set after two failed starts; per-user folder `%LOCALAPPDATA%\plinth\updates\<app id>`, `PLINTH_UPDATES_DIR` in tests). Open: the `prompt` dialog (acts as `next-start`), re-consent for new capabilities, `onUpgrade`, the project core key, web and mobile (UPDATES §9.1). |
| Desktop Hub (`docs/HUB.md`) | H0, H1 (Ed25519 signing), H3 steps 1–3. |
| Web Hub (`docs/web-hub.md`) | The same Hub app in a sandboxed frame with a `plinth:hub` bridge; browse mode; consent; digest and Ed25519 checks. |
| Web export (SPEC §10.3) | `plinth build <dir> --target web [--single-file]`; `web/plinth.js` is GENERATED (`node scripts/gen-plinth-js.mjs`; commit it). |
| Validation apps | 7GUIs 1–7, Pong, primitives, files-demo, and 20 other examples. |
| Platforms | Windows: everything. Linux and macOS: compiler, cores, runner, hub and registry tests pass; no GUI window yet. Mobile: not started (decided: gpui + wasmi on iOS). |
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
| `wit/plinth/app.wit` | Guest/host interfaces: `ui`, `dev`, `error`, `time`, `store`, `clipboard`, `dialog`, `net`, `hub`, `files`. |
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
- Cores: inside a major version only add functions; each addition increments `CORE_MINOR` (and resets `CORE_PATCH`), gets an `ADDED_IN` entry, and needs every host (desktop runner, both engines, AND `web/plinth-web.js`) to supply new WIT functions. A fix with no new function bumps `CORE_PATCH`. Prefer generated code over new runtime functions. 1.12 is not shipped yet: add to it, do not create 1.13 until the owner ships 1.12.
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
- Headless Edge: use the DevTools protocol; it cannot shrink below about 500 px; it sometimes drops the first click into a new iframe; `run-a11y.mjs` has a 60 s limit per DevTools call. A killed run leaves Edge helper processes. The primitives check timed out at page load one time in three runs; run it again.
- AccessKit bounds in `plinth-shoot` tests are device pixels (scale 2 headless): divide by `window.scale_factor()`.
- Live GUI clicks from an agent session often do not reach the window; prefer headless tests.
- **Timings are noisy while other builds run** (agents). Compare variants interleaved in one process (switch draw by draw), not in separate runs.
- gpui-ce: `taffy.rs` `compute_layout` computes a subtree again when an element asks for layout as a root, and the Taffy tree is built again each frame (GAPS F1). Agents in `U:\plinth-wt\` build from the same gpui-ce checkout through a junction: do not change gpui-ce while agents run.

## 8. Working conventions (from the owner)

- Commit after each tested step. No GPG signing; no attribution lines.
- The owner may edit `SPEC.md`; diff it before you change it.
- **Parallel agents** (the owner allows Sonnet subagents): worktrees `U:\plinth-wt\<name>` (`git worktree add U:/plinth-wt/<name> -b wt/<name> master`; `U:\plinth-wt\gpui-ce` is a junction, so `../gpui-ce` resolves). Brief each agent to `cd` into its worktree in every command, to commit on its branch, never to push, to pass `ci-local.sh full` before it reports, and not to edit `HANDOFF.md`. Give a large task in steps with a clear end point: an agent that sees a multi-day task can stop at once without work. Review the diff, merge locally, run the gate (and `run-a11y.mjs` for UI or web changes) on `master`, then remove the worktree.
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

## 10. Next steps (in order)

`docs/PRODUCTION.md` §16 lists the Beta items to do first; they come before the items below. In progress at the end of this session: R3 step 2 (the web watchdog). Then: C3 (ship and freeze core 1.12, which now has RegExp and notify), the `core = "patch"` defect (GAPS "Core policy `patch`"), notify step 2 (the `plinth:notify` module, a desktop toast, the web), RegExp step 3 (`String.match/replace/split/search`), the gpui-ce one-line fix for text input names (`EditableTextElement::write_a11y_info`, PRODUCTION U3).

1. **wasmi, the rest** (steps 1–2 merged; details in GAPS "wasmi engine (2026-10-07)"): (a) run `--test monkey`, `async_await`, `files`, `hub_api` and `net` with `PLINTH_ENGINE=wasmi` — `files`, `hub_api` and `net` exercise imports that never ran on wasmi; (b) release speed numbers, wasmtime against wasmi (`apps::pong_plays_a_point`, `sevenguis::cells_formulas_propagate_and_find_cycles`, `perf.rs` `list_10k_perf` with `--release --ignored`); (c) check `FUEL_PER_MS` with more workloads (calibrated from two, with a 15x spread); (d) a wasmi job in CI (for example `PLINTH_ENGINE=wasmi` for the errors and language suites); (e) step 3: move the engine-free host state into its own crate, so an iOS build does not depend on wasmtime; (f) find out why opt-level 3 on the `wasmi` crate crashes a Windows debug build, and report it upstream if it is a wasmi bug.
2. **Updates, the rest** (UPDATES §9.1; the prompt and re-consent are done): `onUpgrade(fromVersion)`, then the web service worker cache, `apply = "live"`, mobile.
3. **gpui-ce layout fix (GAPS F1):** make nested flex layout linear in the owner's fork (when no agent builds), then remove other needless flex boxes in `render.rs`; check with `frame_cost.rs` and the screenshots.
4. **Smooth motion, the rest:** host-side transitions (UI-ADVANCED U2 motion), a release build for play (say it in `docs/getting-started.md`). Ask the owner to play Pong first.
5. **Web robustness:** done except the watchdog (R3 step 2, in progress).
6. **Pointer, the rest (GAPS G4, 7G-6, 7G-7):** a secondary button or `onContextMenu`; children in `Dialog`; a modal overlay that blocks pointer events on the desktop; `onDrag` and drop targets.
7. **Game gaps:** sound (G7), scale to the window (G9), transitions and springs.
8. **Storage (`docs/STORAGE.md` §6):** a `vault` space and the Hub UI for spaces; a folder as a space; `watch`; sync. Then the notes app (VALIDATION V2).
9. **Experience (`docs/EXPERIENCE.md` §7):** Try and Keep, the Privacy page.
10. **Compiler:** smaller async code (move the promise machinery into the core, GAPS "Async code size"); R3 (chains longer than 1000 terms).
11. **Platforms:** a GUI window on Linux (WSLg is available) and macOS; then mobile with gpui and wasmi.
12. **Distribution:** publish the npm packages and a first registry; Hub H2 (signed indexes); key rotation and the project key.
