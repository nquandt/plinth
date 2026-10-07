# Handoff: Plinth

This file gives a new agent what it needs to continue the work. Read it first. Then read:

- `SPEC.md`: the design (ASD-STE100 Simplified Technical English; keep the style; status notes are inline, search "Status").
- `CONTRIBUTING.md`: commands.
- `docs/HUB.md` (the App Hub), `docs/REGISTRY.md` (registries), `docs/web-hub.md` (the web Hub).
- `docs/GAPS.md`: known language gaps.
- Design drafts from 2026-10-06: `docs/VALIDATION.md` (apps that test Plinth), `docs/STORAGE.md` (profiles, spaces, instances, sync, isolation), `docs/UI-ADVANCED.md` (Level 2 styled primitives on gpui), `docs/EXPERIENCE.md` (user scenarios, UI words, defaults, build order), SPEC §10.3 (web export, `<plinth-app>`, iframes).

## 0. Read this first

**Do not push.** The owner's GitHub Actions minutes are used up (2026-10-06). Keep commits local on `master`. The only gate is a local pass of `bash scripts/ci-local.sh full` (CI runs the same script). For web changes, also run `node web/test/run-a11y.mjs`. When minutes come back, ask the owner before a push and push in one batch. The owner wants self-hosted runners later.

State at this handoff: `master` is about 100 commits ahead of `origin/master`. `ci-local.sh full` and `run-a11y.mjs` pass on it.

**Owner decisions (keep them):**

1. **One implementation for all hosts.** One `.plnt` runs on every host; a host only supplies host APIs and host-owned UI (consent window, error banner). Never build a second UI for one host. (The first web Hub was a hand-written page; it was replaced by the Hub app for this reason.)
2. **Mobile hosts render with gpui.** Only the web host renders to the DOM (SPEC M5, Q1).
3. **Every web app runs in a sandboxed iframe** (opaque origin); the parent page is the host shell (SPEC §10.3).
4. **Advanced UI:** typed style props, no raw pixels for now, theme tokens; the semantic controls get rebuilt on Level 2 primitives later (`docs/UI-ADVANCED.md` §8). The runtime owns all controls (SPEC Q4).
5. **Data belongs to the user:** "your device or your storage, never the vendor's"; strong walls between apps; lasting cross-app access is deferred (`docs/STORAGE.md` §3).
6. **Plain UI words and safe defaults** (`docs/EXPERIENCE.md`).

## 1. What Plinth is

Plinth is a framework for cross-platform apps. Authors write a strict subset of TypeScript with JSX ("Plinth TS"). The compiler turns an app into a `.plnt` package that holds **only app code** (a counter app is about 2 KB) and is the **same file on every platform**. A user installs a host one time: the `plinth` CLI, the `plinth-host` runner, or the Plinth Hub. The host has **runtime cores** (`plinth-rt` builds, Wasm, versioned like nvm), links each app into the core that it needs, runs it in wasmtime (desktop) or the browser (web), and renders a semantic, adaptive UI with `gpui-ce` (the owner's fork at `../gpui-ce`) or the DOM. Apps declare intent; the runtime owns layout, spacing and color. A host proves from an app's imports which capabilities the app can use, and the user approves them (`docs/HUB.md` §1.1).

## 2. State

| Area | State |
|---|---|
| Packages and cores (SPEC §10.4, §10.5) | Core **1.10**. An app declares the lowest core that has the functions it imports. |
| UI API (SPEC §6.3, `docs/ui.md`) | **1.7** (partial styles `hover`/`active`/`focus`/`compact`/`regular`/`wide`); **1.6** (Level 2 primitives `Box`, `Span`, `Pressable`, `Scroll`, `docs/UI-ADVANCED.md` U1). **1.5**: about 30 controls (Checkbox, TextArea, Slider, NumberField, Picker, Progress, Badge, Tabs, Sheet, Dialog, Menu, Grid, Action, Image, Icon, DatePicker, Chart bar/line/pie, Row `trailing`, …). Stack navigation, screen actions, AccessKit roles, keyboard use. Charts: real path geometry on the desktop (`crates/plinth-ui/src/chart.rs`), a zero line for negative values, pie legends with percents, on both hosts. |
| Lists | Keyed reconciler with a minimal-moves diff; host-side virtualization above 200 rows. 10,000 rows: about 4 ms per frame on the desktop. |
| Host APIs (SPEC §8.5, `docs/host-apis.md`) | `time`, `store` (kv), `clipboard`, `dialog`, `net`, `hub` (privileged), `error.report`. Async calls use request ids and `completion` events; without a `done` callback they return a `Promise`. Denied calls never trap. The desktop shows a banner for uncaught app errors. |
| Compiler (SPEC §4) | Unions and narrowing, generics, classes with inheritance, `int`, `Map`/`Set` (with `entries`), tuples, nullable boxing, JSON, fragments, `.map()` JSX children, checked casts, many string and array methods (`sort`, `splice`, `fill`, `flat`), `try`/`catch`/`finally`/`throw` (generated, built-in `Error`), `async`/`await` (`check/asyncfn.rs`; `await` in every statement; async methods), the `Promise` API (`then`/`catch`/`finally`, `new Promise`, `Promise.all`, `resolve`/`reject`; `check/promises.rs`), dynamic `Chart` `data` and `series`. Golden tests for every diagnostic. |
| Desktop Hub (`docs/HUB.md`) | H0, H1 (Ed25519 signing), H3 steps 1–3: the Hub app `examples/hub` (Library, App, Discover screens), launches in new windows with in-process consent, updates with re-consent, version pins, publisher blocks, `plinth://` links, `.lnk` shortcuts and Start menu entries, the `plinthw` launcher. |
| Registry (`docs/REGISTRY.md`) | Static or dynamic registries; `plinth registry build|serve`; capability labels and icons in the app list; a static `hub.json` (`--hub-trusted-key`) for the web Hub. |
| Web host (`web/`) | The same `.plnt` and core file in a browser. Semantic HTML, updated in place (one element per node; focus, caret, selection and hover stay; one changed row of 10,000 costs about 1 ms). Stand-alone page `index.html?app=…&core=…` for tests and development. |
| Web export (SPEC §10.3) | `plinth build <dir> --target web` (folder `dist/web/`) and `--single-file` (`dist/<name>.html`). `web/plinth.js` is the web host as one ES module, GENERATED by `node scripts/gen-plinth-js.mjs` (commit it; `run-bundle.mjs` fails when it is old). It defines `<plinth-app src core height>` (`web/plinth-app.js`); the app frame is inline (`srcdoc` holds the bundle), so no CORS, and the single file works from disk. |
| Web Hub (`docs/web-hub.md`) | The same Hub app in the browser (`web/hub.html` + `hub-shell.js`). Browse mode: the library is the registry listing; `web/hub-host.js` implements `plinth:hub` with IndexedDB. A host consent window, then each app in a sandboxed iframe through the reusable frame pair `web/app-frame.js` + `web/frame-host.js` (bridge documented in `web/README.md`): kv per app, `net`, clipboard and dialogs through the page. Digest and Ed25519 checks with WebCrypto. `plinth registry serve --web` is a static server. |
| Tooling | Hot reload with kept signal state, `plinth-shoot` headless screenshots, GC stress mode, `plinth native` single-file executables, VS Code extension (not published), npm packages (not published). |
| Platforms | Windows: everything. Linux and macOS: compiler, cores, runner, hub and registry tests pass; no GUI window opened yet. Mobile: not started. |

Example apps (valid TypeScript, all built by the check script): `counter`, `todo`, `notes`, `calculator`, `settings-gallery`, `contacts`, `timer`, `gallery`, `dialogs`, `budget`, `utility`, `quotes`, `big-list`, `gc-torture`, `hub`.

## 3. Build, run, test

Prerequisites: Rust stable, `rustup target add wasm32-unknown-unknown`, `../gpui-ce` checked out, Node 22, Edge (for the browser tests). Windows is the main platform.

```sh
bash scripts/ci-local.sh full            # THE check (2–3 min with a warm cache)
bash scripts/ci-local.sh full --shoot    # also headless screenshot tests (needs a GPU)
node web/test/run-a11y.mjs               # headless Edge: axe, renderer checks, the web Hub (needs the axe CDN)
node web/test/run-a11y.mjs --typing-only # only the renderer checks; --hub-only: only the Hub; --export-only: only the web export
node scripts/gen-plinth-js.mjs           # regenerate web/plinth.js after a change to a bundled web file
plinth build <dir> --target web [--single-file] [--out <path>]

plinth new <dir> ; plinth dev <dir> ; plinth check|build <dir> ; plinth run <app.plnt>
plinth native <app.plnt|dir> -o app.exe
plinth validate <app.plnt>
plinth core list|install|export
plinth publisher init|show ; plinth sign <app.plnt> ; plinth build --sign
plinth hub add|list|run|ui|open|remove|grants|block|unblock|block-publisher|policy|groups|source|search|install|update|pin|shortcut|register-scheme
plinth registry build <folder> [--with-core] [--hub-trusted-key <id>] ; plinth registry serve <folder> [--web] [--port N]
bash scripts/web-hub-demo.sh             # web Hub with the examples on http://127.0.0.1:8787/
powershell -File scripts/dev-hub.ps1     # desktop Hub: sign a copy of the Hub app, fill the library, start it
plinth-shoot <app.plnt> <out-dir>
```

**The owner's machine:** `plinth` and `plinthw` are installed in `%USERPROFILE%\.cargo\bin` with `cargo install --path crates/plinth-cli --locked --force` (reinstall after CLI changes). A personal publisher key "Nate" exists; `PLINTH_HUB_TRUSTED_KEYS` is a user environment variable with its id. The web demo signs the Hub with a throwaway key in `target/web-hub-demo-key`. A Cloudflare quick tunnel (`cloudflared tunnel --url http://127.0.0.1:8787`) gives a public URL; start it only when the owner asks. While the demo server runs, `target/debug/plinth.exe` is locked: rename it before a rebuild.

Test-only env vars: `PLINTH_CORES_DIR`, `PLINTH_HUB_DIR`, `PLINTH_PUBLISHER_DIR`, `PLINTH_HUB_TRUSTED_KEYS`, `PLINTH_BLESS=1` (rewrite diagnostic goldens), `PLINTH_TRACE_RENDER=1` (frame times).

## 4. Repository map

| Path | What |
|---|---|
| `SPEC.md`, `docs/` | The design and the docs (see the list at the top). |
| `wit/plinth/app.wit` | Guest/host interfaces: `ui`, `dev`, `error`, `time`, `store`, `clipboard`, `dialog`, `net`, `hub`. |
| `wit/plinth/ui-api.toml` | Control, prop, event and enum ids (UI API 1.5). Append only. `node scripts/gen-web-ids.mjs` regenerates `web/ui-api.js`. |
| `std/*.d.ts` | Typings for the `plinth:*` modules and `lib.d.ts`. `std_typings_match` keeps them equal to the compiler. |
| `crates/plinth-rt` | The core (Rust → wasm32, `no_std`). `CORE_VERSION` must equal `CORE_MINOR`. |
| `crates/plinth-link` | Embedded cores, `rt_abi.rs` (append-only, `ADDED_IN`), `link.rs`, `split.rs`, `cores.rs`, `capabilities.rs`. |
| `crates/plinth-compiler` | Parser (oxc), checker (`check/`: `asyncfn.rs`, `promises.rs`, `jsx.rs`, …), lowering, codegen. Tests in `tests/`. |
| `crates/plinth-ui` | Semantic tree, gpui renderer, theme, charts, calendar math. |
| `crates/plinth-runner-wasmtime` | Runner, `Policy`, kv, timers, request queue, net worker, hub backend trait. |
| `crates/plinth-host-desktop` | Host library (`open_app`, `run`, `run_from_hub`, `poll_hub_launches`, consent window) and `plinth-host`. |
| `crates/plinth-hub` | Library, grants, blocks, groups, sources, updates, pins, OS integration (scheme, shortcuts), `HubService`. |
| `crates/plinth-registry` | Registry documents, source client, static generator (incl. `hub.json`), static server with embedded web files. |
| `crates/plinth-package` | `.plnt` zip, manifest, signatures, publisher keys, single-file payloads. |
| `crates/plinth-cli` | `plinth` and `plinthw`, and the `plinth new` template. |
| `crates/plinth-shoot` | Headless screenshots and GPU tests (not a default member). |
| `web/` | Browser host: `plinth-web.js`, `protocol.js`, `dom-renderer.js`, `zip.js`, the frame pair, the Hub shell and backend; node tests and `run-a11y.mjs` in `web/test/`. |
| `editors/vscode/`, `npm/`, `scripts/` | VS Code extension; npm packages; `ci-local.sh`, demo and dev scripts. |

## 5. Pipeline in one paragraph

`compile_ex(fs, capabilities, dev)`: parse → check (types, narrowing, JSX against `controls.rs`, capability checks `PL1007`, lints; async and promise rewrites) → lower (signals, effects, JSX, hot-reload keys in dev) → codegen against `split::app_layout` (runtime functions are imports; table indices relative to `table_base`) → `split::encode_app` writes `app.wasm` with the lowest needed core version → the compiler links it as a host would, so every build tests the load path. A host picks a core (`cores::link_app`), checks the imports, checks reachable ⊆ declared capabilities, verifies the signature, and links. The web host does the same in JavaScript, with the core and the app as side modules.

## 6. Rules to keep

- A `.plnt` holds only app code and is universal.
- Cores: inside a major version only add functions; each addition increments `CORE_MINOR`, gets an `ADDED_IN` entry, and needs every host (desktop runner AND `web/plinth-web.js`) to supply new WIT functions. Parallel branches: the coordinator renumbers at merge. Prefer generated code over new runtime functions.
- Size: the linked runtime of the counter app stays within 60 KiB (`tests/e2e.rs`, SPEC §5.5). No `core::fmt`, `format!`, `HashMap` or Unicode tables in `plinth-rt`.
- New UI is intent props or typed Level 2 style props from tokens; never raw pixels or colors. `ui-api.toml` ids are append-only.
- Denied host calls never trap. `hub.manage` only for packages signed by a trusted Hub key.
- Linear memory and a Rust runtime; the GC runs only between events.

## 7. Gotchas

- Windows linker: test crates run one at a time (the script does it; `CARGO_BUILD_JOBS` 3–6).
- `plinth-link/build.rs` builds `plinth-rt` into `target/rt` and `target/rt-dev`.
- Generated `.plinth/` typings are ignored by git.
- Goldens change when a message changes; re-bless with `PLINTH_BLESS=1 cargo test -p plinth-compiler --test golden` and read the diff.
- Long bash heredocs with Python and backslashes break; put scripts in files in the scratchpad.
- Headless Edge: use the DevTools protocol, not `--virtual-time-budget`; it cannot shrink below about 500 px; close it with CDP `Browser.close` (a plain kill leaves ghost tabs in Windows Alt+Tab); it sometimes drops the first click into a new iframe, so the tests retry.
- Windows PowerShell 5.1 reads files as ANSI by default; use `[IO.File]::ReadAllText(..., UTF8)` in scripts.
- Live GUI clicks from an agent session often do not reach the window; prefer headless tests.

## 8. Working conventions (from the owner)

- Commit after each tested step. No GPG signing; no attribution lines.
- The owner may edit `SPEC.md`; diff it before you change it.
- Parallel agents: sibling worktrees `C:\repos\plinth-wt-<name>` (so `../gpui-ce` resolves; existing ones: `-lang`, `-rtfix`, `-structure`, `-a11y`, `-ci`, `-hostapi`, `-inputs`, all merged), one or two items per agent, a full `ci-local.sh` pass before it reports, never a push. Brief agents in terms of host APIs, not per-host UIs (§0 decision 1). Merge locally, run the script, then report.

## 9. Next steps (in order)

1. (Done 2026-10-06: the web Hub app runs in its own sandboxed frame with the `plinth:hub` bridge. Small rest: minify `plinth.js`.) **Do not spend most time on the web host** (owner, 2026-10-06): the next steps are mostly outside `web/`.
2. (Done 2026-10-06: **7GUIs, tasks 1–5** in `examples/7guis/`, tests in `tests/sevenguis.rs` and `web/test/run-7guis.mjs`, numbers in VALIDATION §6. Open: `docs/GAPS.md` 7G-1 to 7G-5, mainly `TextField.disabled` and `Row` selection.)
3. **Level 2 UI:** U1 (UI API 1.6) and most of U2 (1.7: `hover`, `active`, `focus` and width-class partial styles) are done; `examples/primitives`. Rest of U2: transitions and springs, a `disabled` partial style. Then the gaps that the Pong agent reports (positioning, Canvas = U4, keyboard events), and interned style records (`docs/UI-ADVANCED.md` §4).
4. **Storage, first slice** (`docs/STORAGE.md` §6): `plinth:files` on a private space and a granted `vault` space, local folder and OPFS providers, host sync to rustfs and Azurite in Docker, Hub UI for spaces, isolation tests. Editor: Markdown source plus a `Markdown` display control. Then the notes app (VALIDATION V2).
5. **7GUIs tasks 6–7:** `Canvas` (U4) for Circle drawer; Grid virtualization for Cells.
6. **Experience build order** (`docs/EXPERIENCE.md` §7): plain words and defaults, Try and Keep, the Privacy page, folder sync with encryption and pairing.
7. **Compiler:** the web host error banner; smaller async code; rest patterns in array destructuring (`const [a, ...r] = t`). (Done 2026-10-06: `Promise.race`/`any`/`allSettled`, the sync-in-async `finally` order, tuples with optional and rest elements, `flat(depth)` with `Infinity`.)
8. **Web Hub** (`docs/web-hub.md` §7): PWA, first-use prompts, session grants in browse mode, a notice to the Hub app when the host changes a grant.
9. **Platforms:** a GUI window on Linux and macOS; then mobile (SPEC Q1, Q3).
10. **Distribution:** publish the npm packages and a first registry; Hub H2 (signed indexes, transparency log); key rotation.

**Open decisions for the owner:** the license (Cargo.toml says Apache-2.0; no LICENSE file; "MIT OR Apache-2.0" is the Rust norm); the project Hub key; self-hosted runners; `docs/HUB.md` H-Q4 (consent while an app waits); the open questions in the design drafts (ST-Q*, UA-Q*, UX-Q*).
