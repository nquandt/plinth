# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first. Then read:

- `SPEC.md`: the design (ASD-STE100 Simplified Technical English; keep the style; status notes are inline, search "Status").
- `CONTRIBUTING.md`: commands.
- `docs/ui.md` (all controls, Level 2 primitives, Canvas), `docs/host-apis.md`, `docs/language.md`.
- `docs/GAPS.md`: known gaps, with the sections "Found by 7GUIs tasks 1-5" and "Games (Pong)", which drive the next UI work.
- `docs/HUB.md` (the App Hub), `docs/REGISTRY.md`, `docs/web-hub.md`.
- Design drafts: `docs/VALIDATION.md` (apps that test Plinth), `docs/STORAGE.md` (profiles, spaces, sync, isolation), `docs/UI-ADVANCED.md` (Level 2 UI; status at the top), `docs/EXPERIENCE.md` (scenarios, UI words, defaults, build order).

## 0. Read this first

**Do not push.** The owner's GitHub Actions minutes are used up (2026-10-06). Keep commits local on `master`. The gate is a local pass of `bash scripts/ci-local.sh full` (CI runs the same script) and, for UI or web changes, `node web/test/run-a11y.mjs`. When minutes come back, ask the owner before a push and push in one batch. The owner wants self-hosted runners later.

State at this handoff (2026-10-06, evening): `master` is 139 commits ahead of `origin/master`. `ci-local.sh full` and `run-a11y.mjs` pass on it. The working tree is clean.

**Owner decisions (keep them):**

1. **One implementation for all hosts.** One `.plnt` runs on every host; a host supplies only host APIs and host-owned UI (consent window, error banner, dialogs). Never build a second UI for one host.
2. **Mobile hosts render with gpui.** Only the web host renders to the DOM.
3. **The web host stays DOM** (SPEC M5, reviewed 2026-10-06). A `gpui_web` canvas host is a possible later, experimental second web host; it is not planned now.
4. **Every web app runs in a sandboxed iframe** (opaque origin), the web Hub app too; the parent page is the host shell (SPEC §10.3).
5. **Level 2 UI:** typed style props, spacing units (4 px) and theme tokens, no raw pixels or colors. The runtime owns all controls; the semantic controls get rebuilt on the primitives later (`docs/UI-ADVANCED.md` U5).
6. **No size limit on the runtime** (core, `plinth-rt`). Only app code (`.plnt`) should stay small, and even that is a goal, not a cap for big apps. A tree-shaken app-plus-runtime bundle is a possible later improvement.
7. **Data belongs to the user:** "your device or your storage, never the vendor's"; strong walls between apps; lasting cross-app access is deferred (`docs/STORAGE.md` §3).
8. **Plain UI words and safe defaults** (`docs/EXPERIENCE.md` §2, §4): "Your apps", "Store", "Permissions", "Keep", "Allow: <what it does>".
9. **Balance:** the web host is one part of the system. Do not spend most of the time on `web/`; prefer the compiler, the desktop/gpui host, the UI, storage and platforms.

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package that holds **only app code** (the counter is about 2 KB) and is the **same file on every platform**. A user installs a host one time: the `plinth` CLI, the `plinth-host` runner, or the Plinth Hub. The host has **runtime cores** (`plinth-rt` builds, Wasm, versioned like nvm), links each app into the core that it needs, runs it in wasmtime (desktop) or the browser (web), and renders with `gpui-ce` (the owner's fork at `../gpui-ce`) or the DOM. Level 1 controls declare intent and the runtime owns the look; Level 2 primitives (`Box`, `Span`, `Pressable`, `Scroll`, `Canvas`) give apps their own look from theme tokens. A host proves from an app's imports which capabilities the app can use, and the user approves them.

## 2. State

| Area | State |
|---|---|
| Cores (SPEC §10.4, §10.5) | Core **1.11** (added `files_call` for `plinth:files`). An app declares the lowest core that has the functions it imports. The runtime also skips unchanged prop and text ops (GAPS G8): Pong went from 9 to 1.4 ops per tick. |
| UI API (`wit/plinth/ui-api.toml`, `docs/ui.md`) | **1.10.** Level 1: about 30 semantic controls (1.5), plus `disabled` on TextField/TextArea/DatePicker and `Row.selected` (1.8). Level 2: `Box`, `Span`, `Pressable`, `Scroll` with spacing-unit and token props (1.6); partial styles `hover`, `active`, `focus`, `compact`, `regular`, `wide` (1.7); `position="absolute"` with insets and `onKeyDown`/`onKeyUp` (1.9); `Canvas` with `rect`, `circle`, `line`, `canvasText` in a scaled view space (1.10). Desktop: `crates/plinth-ui/src/{primitives,canvas}.rs`. Web: `primitiveStyle`, generated partial-style classes and SVG canvas in `web/dom-renderer.js`. |
| Host APIs (`docs/host-apis.md`) | `time`, `store` (kv), `clipboard`, `dialog`, `net`, `files` (text files in the app's private space; capability `files.private`, Low; host path checks; desktop: a folder per app identity under `%APPDATA%\plinth\spaces\private\`; web: the page's IndexedDB through the frame bridge), `hub` (privileged), `error.report`. Async calls return a `Promise` (or take a `done` callback). Denied calls never trap. |
| Compiler (SPEC §4) | Unions and narrowing, generics, classes, `int`, `Map`/`Set`, tuples with optional and rest elements, rest patterns in array destructuring, nullable boxing, JSON, fragments, `.map()` children, checked casts, many string and array methods (`join` also on number/boolean arrays, `flat(depth)`), `try`/`catch`/`finally`, `async`/`await`, the Promise API with `race`/`any`/`allSettled`. Golden tests for every diagnostic. |
| Desktop Hub (`docs/HUB.md`) | H0, H1 (Ed25519 signing), H3 steps 1–3: the Hub app `examples/hub`, launches in new windows with in-process consent, updates with re-consent, pins, publisher blocks, `plinth://` links, shortcuts, `plinthw`. |
| Web Hub (`docs/web-hub.md`) | The same Hub app in the browser, in its own sandboxed frame with a `plinth:hub` bridge (`FrameHub` in `web/hub-host.js`). Browse mode (the library is the registry), a host consent window, every app in a sandboxed frame, digest and Ed25519 checks. |
| Web export (SPEC §10.3) | `plinth build <dir> --target web [--single-file]`. `web/plinth.js` is the web host as one ES module, GENERATED by `node scripts/gen-plinth-js.mjs` (commit it; `run-bundle.mjs` fails when it is old). `<plinth-app src core height>`; the app frame is inline, so the single file works from disk. |
| Validation apps | 7GUIs tasks 1–5 in `examples/7guis/` (numbers in VALIDATION §6); Pong in `examples/pong` (Level 2 boxes, absolute positions, held keys); `examples/primitives` (gallery of Level 2 and Canvas); `examples/files-demo`. |
| Tooling | Hot reload, `plinth-shoot` headless screenshots, `plinth native` executables, VS Code extension and npm packages (both not published). |
| Platforms | Windows: everything. Linux and macOS: compiler, cores, runner, hub and registry tests pass; no GUI window opened yet. Mobile: not started. |

Example apps (all type-checked and built by the check script): `counter`, `todo`, `notes`, `calculator`, `settings-gallery`, `contacts`, `timer`, `gallery`, `dialogs`, `budget`, `utility`, `quotes`, `big-list`, `gc-torture`, `hub`, `hub-mini`, `files-demo`, `pong`, `primitives`, `7guis/{counter,temperature,flight-booker,timer,crud}`.

## 3. Build, run, test

Prerequisites: Rust stable, `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Node 22, Edge (for the browser tests). Windows is the main platform.

```sh
bash scripts/ci-local.sh full            # THE check (about 3–5 min with a warm cache)
bash scripts/ci-local.sh full --shoot    # also the headless GPU screenshot tests
node web/test/run-a11y.mjs               # headless Edge: axe, renderer checks, primitives, Pong keys, web export, web Hub
node web/test/run-a11y.mjs --typing-only # renderer checks only; --hub-only; --export-only
node scripts/gen-plinth-js.mjs           # regenerate web/plinth.js after a change to a bundled web file
node scripts/gen-web-ids.mjs             # regenerate web/ui-api.js after a change to ui-api.toml

plinth dev examples/pong                 # a window with hot reload (Pong: Space starts, hold the arrows or W/S)
plinth build <dir> [--target web [--single-file]] ; plinth run <app.plnt> ; plinth native <app> -o app.exe
plinth hub add|list|run|ui|… ; plinth registry build|serve …
bash scripts/web-hub-demo.sh             # web Hub with every example (Pong too) on http://127.0.0.1:8787/
powershell -File scripts/dev-hub.ps1     # desktop Hub with the Hub app and 9 examples (Pong, Primitives, Files demo too)
plinth-shoot <app.plnt> <out-dir>        # headless screenshots at three widths
```

**The owner's machine:** `plinth` and `plinthw` are installed in `%USERPROFILE%\.cargo\bin` (reinstalled 2026-10-06 with `cargo install --path crates/plinth-cli --locked --force`; reinstall after CLI changes). A personal publisher key "Nate" exists; `PLINTH_HUB_TRUSTED_KEYS` holds its id. The web demo uses a throwaway key in `target/web-hub-demo-key`. Start a Cloudflare tunnel only when the owner asks. While a demo server runs, `target/debug/plinth.exe` is locked: rename it before a rebuild.

Test-only env vars: `PLINTH_CORES_DIR`, `PLINTH_HUB_DIR`, `PLINTH_PUBLISHER_DIR`, `PLINTH_HUB_TRUSTED_KEYS`, `PLINTH_BLESS=1` (rewrite diagnostic goldens), `PLINTH_TRACE_RENDER=1` (frame times).

## 4. Repository map

| Path | What |
|---|---|
| `wit/plinth/app.wit` | Guest/host interfaces: `ui`, `dev`, `error`, `time`, `store`, `clipboard`, `dialog`, `net`, `hub`, `files`. |
| `wit/plinth/ui-api.toml` | Control, prop, event and enum ids (UI API 1.10). **Append only.** |
| `std/*.d.ts` | Typings of the `plinth:*` modules and `lib.d.ts`; `std_typings_match` keeps them equal to the compiler (`check/stdlib.rs` name lists). |
| `crates/plinth-rt` | The core (Rust → wasm32, `no_std`). `CORE_VERSION` must equal `CORE_MINOR`. |
| `crates/plinth-link` | Embedded cores, `rt_abi.rs` (append-only, `ADDED_IN`), `link.rs`, `split.rs`, `cores.rs`, `capabilities.rs`. |
| `crates/plinth-compiler` | Parser (oxc), checker (`check/`: `jsx.rs` props and encoders, `stdlib.rs` std functions, `asyncfn.rs`, `promises.rs`, `stmt.rs` patterns), lowering, codegen. `controls.rs` is the control and prop table. Tests in `tests/` (`lang.rs`, `apps.rs`, `sevenguis.rs`, `files.rs`, `golden/`). |
| `crates/plinth-ui` | Semantic tree, gpui renderer (`render.rs`), Level 2 (`primitives.rs`), `canvas.rs`, theme tokens, charts. |
| `crates/plinth-runner-wasmtime` | Runner, `Policy`, kv, timers, request queue, net and files workers, hub backend trait. |
| `crates/plinth-host-desktop` | Host library and `plinth-host`, consent window. |
| `crates/plinth-hub`, `plinth-registry`, `plinth-package`, `plinth-cli` | Library and grants; registries and the static server (embeds `web/`); `.plnt` and signatures; the CLI and `plinth new`. |
| `crates/plinth-shoot` | Headless screenshots and GPU tests (`primitives_layout.rs` checks Level 2 layout and Canvas by AccessKit boxes). |
| `web/` | Browser host (`plinth-web.js`, `dom-renderer.js`, `protocol.js`, `zip.js`, `files.js`), the frame pair (`app-frame.js`, `frame-host.js`, `hub-storage.js`), the Hub shell and backend, `plinth-app.js`, generated `plinth.js` and `ui-api.js`; Node tests and `run-a11y.mjs` in `web/test/`. |

## 5. How a new control or prop goes in (the usual path)

1. Append ids to `ui-api.toml` (bump `version`, and the assert in `plinth-protocol`'s test); `node scripts/gen-web-ids.mjs`.
2. `crates/plinth-compiler/src/controls.rs`: the spec (the `box_props!` macro for Level 2 boxes); a new `PropTy` needs a case in `check/jsx.rs`. Typings in `std/ui.d.ts`; names in `check/stdlib.rs` `UI_NAMES` for new exports.
3. Desktop: `render.rs` dispatch arm (the match is exhaustive) and `primitives.rs` for style props (`Style` record: props, width-class overlay, partial application).
4. Web: `dom-renderer.js` (`makeView`, `primitiveStyle`), then `node scripts/gen-plinth-js.mjs`.
5. Tests on both hosts: a Node test in `web/test/`, a check in `run-a11y.mjs` if layout or input matters, a desktop test (`tests/lang.rs`/`apps.rs` for behavior, `plinth-shoot` for layout), goldens for new diagnostics. Look at the screenshots.

## 6. Rules to keep

- A `.plnt` holds only app code and is universal.
- Cores: inside a major version only add functions; each addition increments `CORE_MINOR`, gets an `ADDED_IN` entry, and needs every host (desktop runner AND `web/plinth-web.js`) to supply new WIT functions. Prefer generated code over new runtime functions (Canvas shapes, partial styles and `join` are compile-time encodings).
- No runtime size limit (§0 decision 6). Keep `plinth-rt` free of `core::fmt`, `format!`, `HashMap` and Unicode tables anyway. The counter's `app.wasm` is guarded at 8 KiB (`tests/e2e.rs`).
- New UI: intent props or typed Level 2 props from tokens; never raw pixels or colors. `ui-api.toml` ids are append-only.
- Denied host calls never trap. `hub.manage` only for packages signed by a trusted Hub key. Host path checks for `files` on both hosts.
- The runtime skips unchanged props and text, except `value` (the user can change it in the host). A test harness must not edit host-side text and wait for it to come back (see `web/test/run-files.mjs`).
- The GC runs only between events.

## 7. Gotchas

- Windows linker: test crates run one at a time (the script does it; `CARGO_BUILD_JOBS` 3–6).
- `plinth-link/build.rs` builds `plinth-rt` into `target/rt` and `target/rt-dev`. A changed runtime keeps its core version if it adds no function: **export the core again** (`plinth core export target/core.wasm`) before Node or Edge tests, else apps can trap with "unreachable". `ci-local.sh full` does it.
- `registry serve --web` serves the web files embedded in the binary: rebuild the CLI before a web Hub test.
- Subagents start their shell in `C:\repos\plinth`, not in their worktree: brief them to `cd` into the worktree in every command.
- Goldens change when a message or a list of names changes (for example `PL4001_unknown_control`, `PL4002_partial_style_key`); re-bless with `PLINTH_BLESS=1 cargo test -p plinth-compiler --test golden` and read the diff.
- Long bash heredocs with Python, backslashes or `\u` escapes break; write the script to a file in the scratchpad, or use the Edit tool.
- The Bash hook blocks raw `grep`/`find` (and then the whole command line): use `rg` or the Grep tool.
- Headless Edge: use the DevTools protocol; it cannot shrink below about 500 px (use `Emulation.setDeviceMetricsOverride`); close it with CDP `Browser.close`; it sometimes drops the first click into a new iframe. A sandboxed `srcdoc` frame may stay in the page process: `openPage` in `run-a11y.mjs` handles both kinds and finds a frame by its `data-app-id`. Scroll an element into view before a mouse hover.
- AccessKit bounds in `plinth-shoot` tests are device pixels (scale 2 headless): divide by `window.scale_factor()`.
- gpui does not accept the role `GenericContainer`: a plain box has no role.
- Live GUI clicks from an agent session often do not reach the window; prefer headless tests.

## 8. Working conventions (from the owner)

- Commit after each tested step. No GPG signing; no attribution lines.
- The owner may edit `SPEC.md`; diff it before you change it.
- Parallel agents: sibling worktrees `C:\repos\plinth-wt-<name>` (so `../gpui-ce` resolves), one or two items per agent, a full `ci-local.sh` pass before it reports, never a push. Merge locally, regenerate `web/plinth.js` on a conflict, run the script and `run-a11y.mjs`, then report. Old merged worktrees still exist (`-a11y`, `-ci`, `-hostapi`, `-inputs`, `-lang`, `-rtfix`, `-structure`); they can be removed.

## 9. Next steps (in order)

1. **Pointer input (UI-ADVANCED U3, GAPS G4):** `onPointerDown`/`onPointerMove`/`onPointerUp` with x and y (in view units on a `Canvas`, in spacing units on a box), then drag. The runtime passes one event value to a callback (`ARG` in `plinth-rt/src/lib.rs`): a callback with two numbers needs either an argument queue in the runtime (a core minor bump) or a struct value. Then 7GUIs task 6, Circle drawer (Canvas, a context menu, a Dialog with a Slider, undo and redo), and a drag control for Pong.
2. **Game and app loop gaps (GAPS "Games"):** an animation-frame timer (G5), a seeded random generator (G6), window visibility and component lifecycle hooks (G10). Rest of U2: transitions and springs. Then interned style records (`docs/UI-ADVANCED.md` §4).
3. **7GUIs task 7, Cells:** Grid virtualization, many signals, a 26 × 100 grid under 16 ms per frame.
4. **Storage, next part (`docs/STORAGE.md` §6):** a granted `vault` space and the Hub UI for spaces; an existing folder as a space (S6); `watch`; then sync to rustfs and Azurite in Docker. Editor: Markdown source plus a `Markdown` display control. Then the notes app (VALIDATION V2).
5. **Experience (`docs/EXPERIENCE.md` §7):** step 1 (plain words) is done. Next: Try and Keep, the Privacy page (permissions with last use, storage size, Remove).
6. **Compiler:** the web host error banner; smaller async code; rest in object patterns; default values in patterns; `cond ? x : undefined` for optional props (GAPS 7G-3).
7. **Web (small doses):** minify `plinth.js` (176 KiB); web Hub PWA and first-use prompts (`docs/web-hub.md` §7).
8. **Platforms:** a GUI window on Linux and macOS; then mobile with gpui (SPEC Q1, Q3).
9. **Distribution:** publish the npm packages and a first registry; Hub H2 (signed indexes, transparency log); key rotation; later a tree-shaken app-plus-runtime bundle.

**Open decisions for the owner:** the license (Cargo.toml says Apache-2.0; no LICENSE file); the project Hub key; self-hosted runners; `docs/HUB.md` H-Q4; the open questions in the design drafts (ST-Q*, UA-Q*, UX-Q*); whether app data keyed by package digest for unsigned apps (a new space for each build) is acceptable.
