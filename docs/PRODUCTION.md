# Production readiness checklist

This checklist tells when Plinth is ready for public use in production apps. Each item has a gate level and a status. Update the status when work merges. Keep the evidence column correct: name the test, the file or the doc that proves the item.

Status of this file: first version, 2026-10-07.

## 1. Gate levels

Plinth goes through three public levels. A level starts only when all items with that gate, and all items of the levels before it, are **Done**.

| Level | Who uses it | What we promise |
|---|---|---|
| **Preview** | Developers who try Plinth and give feedback. | It installs and works on the documented platforms. Breaking changes can occur in each release. No production use. |
| **Beta** | Early adopters with real apps, at their own risk. | No breaking change to the language, the `.plnt` format, the UI API or the host APIs without a deprecation step. Security reports get a fix. |
| **1.0** | The public, for production apps. | Semantic versioning on all public contracts (§2). Rolling support: every released runtime stays available, so old apps keep running. Signed binaries. A documented security process. |

An item with the gate **Later** comes after 1.0. It does not block a level.

Status words:

- **Done**: merged on `master`, tested in the gate, documented.
- **Partial**: some of the work is merged. The "Next" column tells what is missing.
- **Open**: not started.
- **Decide**: the owner must make a decision first (§14).

## 2. Stability contracts

A production app needs contracts that do not break. Each contract needs a written version rule and a test that finds a break.

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| C1 | UI API ids are append-only and versioned | Preview | Done | `wit/plinth/ui-api.toml`, version assert in `plinth-protocol` |
| C2 | Core ABI is append-only (`ADDED_IN`, `CORE_MINOR`, `CORE_PATCH`) | Preview | Done | `crates/plinth-link/src/rt_abi.rs`, SPEC §10.5 |
| C3 | Ship core 1.12 and freeze it (no more additions to a shipped minor) | Beta | Done | `rt_abi::SHIPPED_MINOR` = 12; `crates/plinth-link/tests/core_abi_frozen.rs` with `core-abi.snapshot` (a new function in a shipped minor, or a changed one, fails); the shipped file in `cores/shipped/1.12.0.wasm` with its digest. |
| C4 | `.plnt` container and manifest have a format version and a compatibility rule | Beta | Done | `docs/COMPATIBILITY.md` §4: a newer `plinth-profile` gives "This app needs a newer version of Plinth"; unknown entries are kept (signed, never run, never served); the manifest tolerates unknown fields and enum values at every level; `plinth.toml` stays strict. Tests in `plinth-package`, `plinth-hub` and `web/test/run-hub-integrity.mjs`. Frozen old packages: C8. |
| C5 | Host interfaces (`wit/plinth/app.wit`) have a version and an append-only rule | Beta | Done | `docs/COMPATIBILITY.md` §3; `plinth-link/tests/wit_append_only.rs` with `wit/plinth/app.wit.snapshot` |
| C6 | Plinth TS language version: a list of the accepted syntax and the deviations from ECMAScript | Beta | Done | `docs/COMPATIBILITY.md` §5 (rule: a source that compiles stays compiling); `docs/language.md` "Language version 1.0" (the frozen subset, 2026-10-08). |
| C7 | Deprecation policy: how a feature is marked, how long it stays, what diagnostic the compiler gives | Beta | Done | `docs/COMPATIBILITY.md` §6. The first deprecation adds its `PLW` golden test. |
| C8 | Compatibility test suite: `.plnt` files built by each released version run on the newest host | 1.0 | Open | Keep the released packages of `examples/` in the repo and run them in the gate. |
| C9 | Support policy: rolling support, no guaranteed window; every released runtime core stays available | 1.0 | Done | §15 PR-Q3 (owner, 2026-10-07) |
| C10 | Every released core and host stays available for download, so an old app always finds its core | 1.0 | Partial | Shipped cores are kept in `cores/shipped/` (digest table). Next: a download location for hosts (release assets or a registry) and a test that installs an old core from it. |

## 3. Language and compiler

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| L1 | Core TypeScript subset: unions, narrowing, generics, classes, `Map`/`Set`, tuples, destructuring, JSON, `try`/`catch`, `async`/`await`, Promise API | Preview | Done | HANDOFF §2 "Compiler", `tests/language*` (171 cases) |
| L2 | Every rejected feature gives a diagnostic with a golden test | Preview | Done | `plinth-compiler/tests/golden.rs` (SPEC M3 exit) |
| L3 | The compiler never panics or overflows its stack on any input | Preview | Done | `tests/robustness.rs`, `PL1000`, `stacker` |
| L4 | Regular expressions (`RegExp`) | Beta | Done | Linear-time matcher in the core (`plinth-rt/src/regex.rs`), literals, `new RegExp`, `test`, `exec`, collectable objects; the `String` methods `match`, `matchAll`, `replace`, `replaceAll`, `split` and `search` with a `RegExp` (`plinth-rt/src/regexp.rs`, `check/regexps.rs`). Tests: `tests/lang.rs` `regexp_*` and `regex_*`, `plinth-rt` `regexp::tests`, golden `PL2018_match_unknown_g`. `docs/language.md` "Regular expressions" lists the subset and the differences from JavaScript. Lookaround and backreferences stay out (`docs/LIMITS.md`). |
| L5 | Locale-aware text: number, date and plural formatting (`plinth:locale`) | Beta | Done | Steps 1 and 2 done (`docs/I18N.md` §3.6, §3.7). Step 1: `plinth:locale` `locale()` and `direction()` (core 1.13), reactive, with the locale change event, on the desktop (wasmtime and wasmi: the OS locale, read again when the window comes to the foreground) and the web (the page locale and `languagechange`, in the Hub, a web export and the stand-alone page; `lang` and `dir` on the root element). Step 2: `formatNumber` (decimal, percent, currency, compact), `formatDate` (date and time styles, local time zone), `pluralCategory` (cardinal, ordinal), `formatList` and `compare` (collation), reactive, with ICU4X 2.3 on the desktop and `Intl` on the web. Tests: `tests/locale.rs` and `tests/locale_format.rs` on both engines, `web/test/run-locale.mjs`; the shared cases `tests/locale_cases.txt` (56 calls, 8 locales) give the same text on both hosts, with 8 known differences out of 448 (`docs/I18N.md` §3.7). Message catalogs and `t()` are U8. |
| L6 | Long expression chains (more than 1000 terms) | Beta | Done | 2026-10-08, GAPS R3. A left-deep chain (`a + b + ...`, `&&`, `\|\|`, `??`, `?.`, `x.f().g()...`) has no length limit: the parser, the checker, the async transform, `lower`, codegen, and `Clone`/`Drop` of the trees walk it with a loop. Real nesting keeps the limit of 1000 levels, with the new code `PL1008`; a scan before oxc (`nesting.rs`) stops code that nests far too deep, so no input overflows the stack. Tests: `crates/plinth-compiler/tests/chains.rs` (5,000 terms of sums, string concatenation, mixed operators, `&&`/`\|\|`, `??`, `?.`, `\|\|` on strings and method calls on a builder class run with the right value, and a narrowing `&&` chain of 1,000 terms; 100,000-term sums, `&&` chains, string concatenations, method chains and `await` chains compile; nesting of 9 kinds at 1,001, 15,000 and 100,000 levels gives one `PL1008`), golden `PL1008_too_deep`, `nesting::tests`. Compile time of 100,000 terms in a debug build: sum 2.5 s, `&&` 4.0 s, string concatenation 1.9 s, method calls 3.0 s, `await` and a sum 2.0 s; linear (25,000 to 200,000 terms). `app.wasm` sizes of all examples do not change. A `??`, `?.` or non-boolean `\|\|` chain stops near 50,000 links (one local for each link, GAPS R3). |
| L7 | Smaller async code (promise machinery in the core) | 1.0 | Open | GAPS "Async code size". |
| L8 | Libraries: share Plinth TS code between apps (`hub:<publisher>/<lib>`, `plinth.lock`) | 1.0 | Open | SPEC §12 "Libraries". Without them, each app copies code. |
| L9 | Runtime errors name the source file and the line (source maps for stack traces) | Beta | Done | 2026-10-08. The compiler writes the custom section `plinth-srcmap` into `app.wasm`: for each app function, the line of each code range where the code can stop (a call, a loop, `unreachable`, an integer division), as delta-encoded LEB128 (format: `crates/plinth-link/src/srcmap.rs`). The host link step moves it into the linked module. Desktop (wasmtime): the stop reason ends with the place of the innermost app frame, for example `array index out of bounds (app/lib.ts:3)`; the plain reason stays first. Web: the same, from the `wasm-function[N]:0xOFFSET` frames of the error stack (`trapPlace` in `web/plinth-web.js`). An uncaught `throw` names its place in all builds: `Uncaught Error: boom (app/main.tsx:12)` (a string literal at the `throw`, no new core function). Cost: 3 % to 7 % of `app.wasm` (counter 1,939 B + 91 B; hub 24,264 B + 1,490 B); all size budgets pass. Not done: `plinth build --release` removes the map on purpose (Binaryen changes the code, so the offsets would be wrong), so a release build gives the plain trap reason; wasmi gives no frame offsets, so no trap place there; the web time limit stops the Worker with no stack; lines only, no columns; an uncaught rejection of a `Promise` has no place. Tests: `crates/plinth-compiler/tests/errors.rs` (wasmtime and wasmi), `web/test/run-stop.mjs`, `web/test/run-worker.mjs`, `crates/plinth-cli/tests/release_build.rs`. |

## 4. Runtime and engines

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| R1 | GC and allocator have no known leaks; memory stays level under random use | Preview | Done | `tests/monkey.rs`, GAPS "Robustness" |
| R2 | Desktop limits: time limit, memory cap, stack limit stop only the app, with a plain reason | Preview | Done | `tests/errors.rs` "Limits" |
| R3 | Web limits: a time limit (Worker and watchdog) and a memory cap | Beta | Done | Memory cap: `capCoreMemory` (`web/test/run-stop.mjs`). Time limit: each app runs in its own Worker, and a watchdog ends it after 2 s in one call (`web/app-worker.js`; `web/test/run-worker.mjs`, `checkStopBanner` in `run-a11y.mjs`). |
| R4 | wasmi engine passes all suites (monkey, async, files, hub, net) and runs in CI | Beta | Done | 2026-10-08: every compiler suite that runs apps passes with `PLINTH_ENGINE=wasmi` (`files`, `hub_api`, `net` for the first time; `monkey` in a release build); three engine-aware tests (trap place, round-trip bound, start-time budgets). `FUEL_PER_MS` is now 2,000,000 from release measurements of ten workloads (`tests/perf.rs` `engines_speed`). Speed: wasmi is 7 to 11 times slower than wasmtime in a release build. GAPS "wasmi engine" step 3. |
| R5 | Engine-free host state crate (no wasmtime dependency for iOS) | 1.0 | Partial | 2026-10-27, Windows part done: `wasmtime` is an optional dependency of `plinth-runner-wasmtime` (feature `wasmtime`, on by default); the wasmi engine and `HostState` need no wasmtime type (`host_traits.rs` has the host traits by hand). `cargo tree` of the wasmi-only build has no wasmtime; its tests (77) and the `secrets` and `notify` suites pass with and without `PLINTH_ENGINE=wasmi`. A Rust fault found for iOS is fixed (`time` with `num_threads` does not build there). Open: `cargo check --target aarch64-apple-ios` stops at `ring` and `libsqlite3-sys` here (no Apple SDK), so the runner code is not type-checked for iOS yet; `plinth-hub` and `plinth-registry` still pull wasmtime through the default features (`docs/MAC-HANDOFF.md` §5.1, §5.2). Needed for iOS (PR-Q1). |
| R6 | Performance budgets in the gate: start time, frame cost, memory per app, `.plnt` size | Beta | Done | `crates/plinth-compiler/tests/budgets.rs` with `tests/budgets.toml` (in `ci-local.sh full` and `nogui`): `app.wasm` size of each example, memory after `init` and after 300 UI events, start time (load and init) for five examples; the core size is recorded, not limited (SPEC.md §5.5). Frame cost of each example: `crates/plinth-shoot/tests/frame_cost.rs` with `tests/frame_budgets.toml` (`--shoot`). Bless with `PLINTH_BLESS=1`. Budgets and measurements: `docs/guides/performance.md` §9. |
| R7 | Nested flex layout is linear (gpui-ce fix) | Beta | Open | GAPS F1; HANDOFF §10 item 3 |

## 5. UI and accessibility

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| U1 | Level 1 controls and Level 2 primitives, same tree on desktop and web | Preview | Done | `docs/ui.md`, UI API 1.13 |
| U2 | Web accessibility: axe checks pass on all examples | Preview | Done | `web/test/run-a11y.mjs` |
| U3 | Desktop accessibility: AccessKit tree with names and roles for every control | Beta | Partial | `plinth-shoot/tests/a11y_controls.rs` walks every Level 1 control and Level 2 primitive (role, name, value, state, disabled, expanded, selected; each node with a focus or click action has a name). Same semantics as the web except: a `Picker` with more than 4 options (desktop list box, web `<select>`), `DatePicker` (desktop button and grid dialog, web native input), the selected `Row` and the current screen (desktop `selected` / no current state, web `aria-current`), `Scroll` (a tab stop only on the web), a text field's name (desktop: on the group around the input, a gpui-ce limit). Next: a manual check with Narrator, VoiceOver and Orca; the text input name in gpui-ce. |
| U4 | Keyboard: every action is possible without a pointer; visible focus | Beta | Partial | Desktop: `plinth-shoot/tests/a11y_controls.rs` (Tab and Shift+Tab order, Enter and Space, typing, focus ring in a screenshot, Escape on menus, popovers, Sheet and Dialog, focus trap in modals, arrow keys in Picker, Tabs, Slider, NumberField, DatePicker, Menu). Web: native elements and the menu and tabs key handlers. Next: a keyboard-only test of each example on both hosts, and the same fixture in `run-a11y.mjs`. |
| U5 | Text input: IME composition, on desktop too | Beta | Open | The web uses native inputs. Check the gpui-ce text field with a CJK IME. |
| U6 | Pointer: secondary button or `onContextMenu`, drag and drop, modal overlay that blocks pointer events | Beta | Done | UI API 1.13, on the desktop and the web. Part 1: `onContextMenu(x, y)` on boxes, `Canvas` and `Row` (a right-click, a long press on the web, the menu key or Shift+F10 on the focused element); a `Menu` with `open` shows at the point; a modal layer blocks the pointer (GAPS G4). Part 2: drag and drop in one app: `List` `onReorder(from, to)` (a drag of a row, or Alt+Up/Down on the focused row), `dragValue` and `onDrop(value, x, y)`; the host draws the drop target and the drop line and tells the result in a live region; a modal layer stops a drag; a finger drags after a long press on the web; children (a small form) in `Dialog` (GAPS 7G-7). Evidence: `plinth-shoot/tests/context_menu.rs` and `drag_drop.rs` (desktop: reorder by drag and by Alt+Up/Down, a drop with its value, the drop highlight in the pixels, a modal stops a drag, a Dialog with a slider takes the focus and the value); `checkContextMenu` and `checkDragDrop` in `run-a11y.mjs` (Edge: the same, with a mouse and a finger, in the page and in the sandboxed app frame); compiler `frontend.rs` `drag_and_drop_and_dialog_children`. Used by the 7GUIs circle drawer (GAPS 7G-6, 7G-7) and `examples/todo`. Open (GAPS G4): drag and drop between apps and with the operating system (files), auto-scroll during a drag, `Grid` reorder. |
| U7 | System settings: dark mode, high contrast, text scale, reduced motion | Beta | Done | On the Windows desktop and the web, the runtime theme follows the four settings, at start and when they change, with no app code. Windows desktop: dark mode from the window appearance (gpui-ce); high contrast from `SPI_GETHIGHCONTRAST` with the OS high-contrast colors (`GetSysColor`); the text size from `TextScaleFactor` (100 % to 225 %, it sets the rem size); reduced motion when "Animation effects" is off (`SPI_GETCLIENTAREAANIMATION`). The host reads them again each second (`crates/plinth-host-desktop/src/system_settings.rs`). Web: `prefers-color-scheme`, `prefers-contrast: more`, `forced-colors: active`, `prefers-reduced-motion: reduce` and the browser font size (all type is in rem). Overrides for tests: `PLINTH_THEME`, `PLINTH_HIGH_CONTRAST`, `PLINTH_TEXT_SCALE`, `PLINTH_REDUCED_MOTION`. Evidence: `plinth-ui` `theme.rs` tests (WCAG ratio of each high-contrast text and background pair, at least 7:1; the overrides; the reduced-motion flag); `plinth-shoot/tests/system_settings.rs` (three examples in dark and high contrast, 200 % text with no overflow or overlap, a live change redraws the open window); `checkSystemSettings` in `run-a11y.mjs` (emulated media, axe in dark and high contrast, a 32 px default font size). Linux and macOS: `docs/PLATFORM-TODO.md`. Open: no API for an app to read the settings (a `Canvas` game that draws its own motion, `docs/LIMITS.md`). |
| U8 | Localization: app strings in more than one language, the user's locale | 1.0 | Open | Design draft: `docs/I18N.md` (catalogs, `t()`, host formatting with ICU4X and `Intl`). Steps 1 and 2 are done (L5). Next: answer I18N-Q1..Q5, then steps 3 and 4. |
| U9 | Right-to-left layout | 1.0 | Open | `docs/I18N.md` §3.5. Risk: bidirectional text in gpui-ce (I18N-Q1). |
| U10 | Motion: host-side transitions and springs | 1.0 | Open | UI-ADVANCED U2; HANDOFF §10 items 4, 7 |
| U11 | Sound | 1.0 | Open | GAPS G7. 1.0 needs it (PR-Q2). |

## 6. Host APIs

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| H1 | `time`, `store` (kv), `clipboard`, `dialog`, `net`, `files`, `error.report` on all hosts | Preview | Done | `docs/host-apis.md` |
| H2 | Denied calls never trap; distinct denial reasons | Preview | Done | SPEC §11 |
| H3 | Notifications (`notify`) | Beta | Partial | Step 1 done: WIT `notify` (`show`, `cancel`), core 1.12 functions, the `notify` capability and policy, a `NotifyBackend` with a test backend on wasmtime and wasmi (`tests/notify.rs`). Step 2a done: the `plinth:notify` module in the compiler (`show`, `cancel`, `lastError`; `std/notify.d.ts`), a Windows toast in the desktop host, `examples/notify-demo`. Step 2b done: the web host (the page shows the notification with the browser's `Notification` API for the app frame; `web/test/run-notify.mjs`, `run-a11y.mjs`). Next: Linux and macOS desktop backends. |
| H4 | Structured storage (`store.sql`) | Beta | Done | Owner decision 2026-10-07. Design: `docs/SQL.md` (SQLite on every host; on the web in the parent page with OPFS; async API with typed rows; core 1.13). Step 1 of 5 is done: the desktop engine (`crates/plinth-runner-wasmtime/src/sql.rs`: `rusqlite` with the bundled SQLite and FTS5, the authorizer, the time, size and result limits, a worker thread, unit tests). Step 2 of 5 is done: apps call `sql.exec`, `sql.query<T>`, `sql.batch` and `sql.migrate` from `plinth:store` on the desktop (wasmtime and wasmi), with core 1.13, the capability `store.sql` (Low) and the warning `PL2026` (`crates/plinth-compiler/tests/sql.rs`). Step 3 of 5 is done: the web engine (`web/sql.js`: the official SQLite Wasm build in a Worker of the page, with OPFS; the same limits and error texts; `web/test/run-sql.mjs` runs the shared cases of `sql_cases.txt` that `tests/sql.rs` runs on the desktop). Step 4 of 5 is done: `examples/notes` keeps its notes in a table and searches them with FTS5 (a prefix search on each word, with the user's text escaped), moves old kv data one time, and has tests with 1,000 notes on both hosts (`crates/plinth-compiler/tests/notes.rs`, `web/test/run-notes.mjs`). Step 5 of 5 is done: `docs/guides/storage.md` (when to use `sql`, an example that passes `plinth check` and `tsc`, rules, errors, limits, where the data is), `docs/host-apis.md`, `docs/LIMITS.md`. The folder form of a web export has `store.sql` too (`run-a11y.mjs` checks the notes export across a reload). Known limits (`docs/SQL.md` §8, §9, `docs/LIMITS.md`): a single-file web export has no SQL engine (`plinth build` warns); on the web one tab of an origin at a time; no `BLOB`; no row-level sync. |
| H7 | Secrets (`plinth:secrets`) | Beta | Partial | `docs/SECRETS.md`. Done 2026-10-27: WIT, core 1.13, capability, compiler, Windows Credential Manager, web (WebCrypto and IndexedDB), tests on both engines. Done 2026-10-27: removal on uninstall (`plinth hub uninstall` calls `remove_app_secrets`; the web `HubStore.uninstall`), a data quota, and export without secrets (`docs/STORAGE.md` §7). Open: macOS and Linux check, iOS Keychain, an uninstall button in the Hub UI and a `plinth:hub` call. |
| H5 | Storage: `vault` space, a folder as a space, `watch` | 1.0 | Open | `docs/STORAGE.md` §6 |
| H6 | Sync between devices of one user | Later | Open | STORAGE §6. Not in 1.0 (PR-Q5). |
| H7 | Each host API has a documented behavior on each host, with the same test on each host | Beta | Done | `docs/host-apis.md` "Support on each host" (behavior and tests: desktop wasmtime, desktop wasmi, web). No "no test" cell is left. `clipboard` (write and read, all three hosts): `crates/plinth-compiler/tests/clipboard.rs`, wasmi gate, `web/test/run-clipboard-dialog.mjs`. `dialog` on the web: `web/test/run-clipboard-dialog.mjs` (2026-10-08, Q4). |

## 7. Security

Production apps run code from many publishers. This area must be complete before 1.0, and a review by a third party is necessary.

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| S1 | Default deny; capabilities proved from imports; user consent | Preview | Done | SPEC §11, consent window |
| S2 | Every web app in a sandboxed iframe with an opaque origin | Preview | Done | SPEC §10.3 |
| S3 | `net` blocks private network ranges unless `net.local` | Preview | Done | All non-public IPv4/IPv6 ranges and embedded IPv4 on both hosts (`policy.rs` `is_private_ip`, `plinth-web.js` `isPrivateNetHost`, `web/test/run-net-private.mjs`); the desktop also checks the address after DNS resolution (`net.rs` `PublicOnlyResolver`, `tests/net.rs`). The web cannot check DNS results (THREAT-MODEL §6 item 1). |
| S4 | Packages and update feeds are signed (Ed25519); no downgrade | Preview | Done | `plinth-package::feed`, desktop updater |
| S5 | Re-consent when an update asks for new capabilities | Beta | Done | `plinth-host-desktop::updater` (`update_consent`, `prepare_start`), the Hub (`docs/HUB.md` §7.3 step 3); UPDATES §6 |
| S6 | Project key that signs cores and the Hub; key rotation and revocation | Beta | Open | Owner decision 2026-10-07: an offline root key that the owner holds signs a release key for CI; the root key rotates or revokes the release key. |
| S7 | `SECURITY.md`: how to report a vulnerability, response time | Beta | Done | `SECURITY.md`. Turn on GitHub private vulnerability reporting when the repo is public (G6). |
| S8 | Dependency audit in CI (`cargo deny` or `cargo audit`, `npm audit`) | Beta | Done | `deny.toml`; `bash scripts/ci-local.sh deny` (also in `full`; Linux CI); `docs/DEPENDENCIES.md`. Two wasmtime advisories are ignored with a reason until the wasmtime update (DEPENDENCIES §4 A1). |
| S9 | Fuzz tests run in CI on a schedule with more iterations | Beta | Partial | `robustness.rs`, `hostile_ops.rs`, `hostile_render.rs` run in the gate with few iterations. |
| S10 | Threat model document: app vs app, app vs host, publisher vs user, network | Beta | Done | `docs/THREAT-MODEL.md`. Update it with each new host API. |
| S11 | Security review by a third party (sandbox, consent, signing, web bridge) | 1.0 | Open | |
| S12 | Hub: signed indexes (H2) and a transparency log | 1.0 | Open | `docs/HUB.md` |
| S13 | The Hub tells the user when an app's pinned core has a known vulnerability | 1.0 | Open | THREAT-MODEL §6 item 6; needs an advisory list in the core feed. |
| S14 | The signature covers the manifest | Preview | Done | `plinth.signature/2` digests the raw bytes of `manifest.toml` and every other entry (`docs/HUB.md` §6.1, THREAT-MODEL §4.4). Desktop: `plinth-package` `signature::tests` (a changed `version`, `id`, capability, feed, `core` or profile is refused; `write` puts the signed bytes in the file). Web: `web/test/run-hub-integrity.mjs`. |

## 8. Platforms

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| P1 | Windows: compiler, host, Hub, native export | Preview | Done | |
| P2 | Linux: a GUI window (gpui), same tests as Windows | 1.0 | Open | HANDOFF §10 item 11. Compiler and runner tests pass. |
| P3 | macOS: a GUI window (gpui), same tests as Windows | 1.0 | Open | HANDOFF §10 item 11 |
| P4 | Web: Chrome, Firefox and Safari in the test run (now Edge only) | Beta | Open | SPEC M5 exit |
| P5 | Windows and macOS binaries signed and notarized | Beta | Open | `docs/RELEASING.md` "What is not released yet" |
| P6 | One `.plnt` runs the same on all three desktop OSes; native exports start with a double-click | Beta | Open | SPEC M4 exit |
| P7 | Android (gpui, wasmtime) | 1.0 | Open | PR-Q1: 1.0 covers all platforms. SPEC M6 |
| P8 | iOS (gpui, wasmi) and the iOS distribution model | 1.0 | Open | PR-Q1: 1.0 covers all platforms. SPEC Q3 |

## 9. Updates and distribution

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| D1 | npm packages: `@plinth/cli`, platform packages, `create-plinth` | Beta | Partial | `npm/`, `release.yml`. Next: publish them; test `npm create plinth` on all three OSes. |
| D2 | Release pipeline tested on all platforms, with a changelog | Beta | Partial | `docs/RELEASING.md`. Only Windows is tested; no changelog step. |
| D3 | Desktop self-update: signed feed, rollback, no downgrade | Preview | Done | `plinth-host-desktop::updater` |
| D4 | Update `prompt` dialog and `onUpgrade(fromVersion)` | Beta | Done | The desktop restart prompt (UPDATES §5). `onUpgrade(fromVersion)` (core 1.13, 2026-10-08) on the desktop (both engines) and the web (stand-alone, the Hub, `<plinth-app>`): `crates/plinth-compiler/tests/upgrade.rs`, `web/test/run-upgrade.mjs`; the host rule in `docs/migrations/core-1.13.md` §3. UP-Q5 has an interim rule (UPDATES §6). |
| D5 | Web updates (service worker cache) | Beta | Open | UPDATES §9.1 |
| D6 | A public registry and the Hub app on desktop and web | Beta | Partial | `docs/HUB.md`, `docs/REGISTRY.md`, `docs/web-hub.md`. Next: host a first registry. |
| D7 | Update policy decisions (UP-Q1..Q6) | Beta | Partial | Decided 2026-10-07: UP-Q2 `apply = "live"` after 1.0; UP-Q3 feeds at the author's URL and in registries that the user adds, no Plinth-run service; UP-Q4 the key model of S6. Open: UP-Q1 (default policy stays `follow`), UP-Q5, UP-Q6. |

## 10. Developer tools

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| T1 | `plinth new`, `dev` (hot reload), `check`, `build`, `run`, `native` | Preview | Done | SPEC §13 |
| T2 | VS Code extension: diagnostics, completion | 1.0 | Partial | `editors/vscode`. Next: publish it to the Marketplace. |
| T3 | A test kit for app authors: render a screen, send events, check the tree | Beta | Partial | Step 1 done (2026-10-08), design and API in `docs/TESTING.md`: tests in Plinth TS (`app/**/*.test.tsx`, module `plinth:test`), run by `plinth test [dir] [--filter]` (and `npm test` in a new project). `render(<X />)`, `renderApp()`, queries by label, title, text and role (an ambiguous match fails and lists the matches), `press`, `type`, `submit`, `toggle`, `select`, `moveUp`/`moveDown`, `showScreen`, `tick` and `settle` on a test clock, `expect` with query and value matchers and `not`. Each test runs in a new instance with a new kv store and private space. The test code runs in the guest and talks to the runner over `dev.log` and UI events: no core function and no WIT item for tests. A failure gives the line (the compiler fills `__at`) and the screen as text. Tests: `crates/plinth-cli/tests/plinth_test.rs`; `examples/todo` (8 tests) and `examples/counter`; the `plinth new` template; `ci-local.sh full` runs `plinth test` on the examples with tests. Step 1 part of TESTING §12 is also done (2026-10-08): `describe`, `beforeEach`/`afterEach`, `testOnly`/`testSkip` (and `describeOnly`/`describeSkip`; Plinth TS has no function with properties, so no `test.only`), the matchers `toBeGreaterThan`/`toBeGreaterThanOrEqual`/`toBeLessThan`/`toBeLessThanOrEqual` and `toThrow`, and `plinth test` on wasmi (`PLINTH_ENGINE=wasmi`, a new `wasmi` cargo feature of `plinth-cli`). Next (TESTING §12): the same tests on the web host, dialog answers, fake network answers, pointer and key steps, deep `toEqual`. |
| T4 | A debugger or an inspector: the UI tree, signals, host calls | Beta | Open | |
| T5 | A profiler view: frame times, GC, memory (`PLINTH_TRACE_RENDER` as a start) | 1.0 | Open | |
| T6 | `plinth publish` to a registry | Beta | Partial | SPEC §12, `plinth hub` |
| T7 | `plinth build --release` optimizes the app module with Binaryen `wasm-opt` (and the release core) | Beta | Partial | App module done (2026-10-07): `plinth build --release` runs Binaryen `-Oz` (the `wasm-opt` crate, Binaryen 116, built into `plinth`; `crates/plinth-cli/src/optimize.rs`). Over the 26 example apps: 160,745 B to 138,863 B (-13.6 %), gzip -11.5 %; counter 1,939 B to 1,674 B. No measurable speed change (`tests/perf.rs`, the core does the work). The tool checks the result (imports, custom sections, no exports, a link into the core) and keeps the compiler's module if a check fails. Tests: `crates/plinth-cli/tests/release_build.rs` (smaller, the same op stream as the debug module for 6 examples, deterministic bytes). All compiler suites (lang 193, errors, async, monkey, e2e, apps, 7GUIs) and 9 web Node tests passed with optimized modules. Next: the core. `-Oz` makes the 1.12 core 131,188 B to 112,696 B (-14.1 %, gzip -4.9 %) and all suites pass with it, but 1.12 is shipped and frozen; do it in `plinth-link/build.rs` for 1.13 (`docs/guides/performance.md` §8). |

## 11. Documentation

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| X1 | Getting started from zero to a running app | Preview | Done | `docs/getting-started.md` |
| X2 | Reference: language, UI, host APIs | Preview | Done | `docs/language.md`, `docs/ui.md`, `docs/host-apis.md` |
| X3 | A public docs website with search | Beta | Open | |
| X4 | Guides: storage, network, updates, publishing, accessibility, performance | Beta | Done | `docs/guides/` (index `docs/guides/README.md`); each example checked with `plinth check`. |
| X5 | A migration guide for each breaking change and each core minor | Beta | Done | `docs/migrations/` (the index and how to write a guide) and `docs/migrations/core-1.13.md` (app authors, host authors, users). `crates/plinth-link/tests/migration_guides.rs` fails when a core minor after 1.12 has no guide or the index does not list it. A breaking change gets its guide with the new major version (`docs/COMPATIBILITY.md` §6). |
| X6 | `CHANGELOG.md` | Preview | Done | `CHANGELOG.md`. Add an entry for each change that authors or users see. |
| X7 | Known limits page (what Plinth cannot do yet) | Preview | Done | `docs/LIMITS.md`. Update it when a limit goes away. |

## 12. Quality gates and CI

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| Q1 | Local gate: `scripts/ci-local.sh full --shoot`, `run-a11y.mjs` | Preview | Done | HANDOFF §3 |
| Q2 | CI runs on each push (self-hosted Windows and Linux runners online) | Beta | Partial | HANDOFF §0.2 item 1 (owner steps open) |
| Q3 | CI on macOS for each release, not only weekly | Beta | Open | |
| Q4 | A wasmi job in CI | Beta | Done | `bash scripts/ci-local.sh wasmi` (the runner tests and 21 app suites on wasmi, about 4 min warm) and the CI job "Windows (wasmi engine)" (self-hosted Windows runner, on a push to `master` and by hand). `lang`, `sevenguis`, `monkey` and `budgets` are too slow in a debug wasmi build for the gate; run them by hand (GAPS "wasmi engine"). |
| Q5 | A release is blocked when the gate fails on any platform | Beta | Open | `release.yml` |

## 13. Validation with real apps

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| V1 | 7GUIs tasks 1–7 | Preview | Done | `docs/VALIDATION.md` |
| V2 | Flagship notes app (storage, offline, search) | Beta | Open | VALIDATION V2; depends on H4, H5 |
| V3 | App suite in one host (isolation, fairness, memory) | Beta | Open | VALIDATION V3 |
| V4 | At least three apps by developers outside the project, used by real users for a month | 1.0 | Open | Record each gap they find in `docs/GAPS.md`. |

## 14. Project and legal

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| G1 | License: MIT OR Apache-2.0 | Preview | Done | `LICENSE-MIT`, `LICENSE-APACHE`, `Cargo.toml`, `package.json` files |
| G2 | Final project name and npm scope | 1.0 | Open | SPEC Q2. "Plinth" is a placeholder; the npm name `plinth` is taken. | The owner decides at the end. |
| G3 | `CONTRIBUTING.md` for outside contributors; code of conduct | Beta | Done | `CONTRIBUTING.md` ("A pull request"), `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1). Add a private contact when the repo is public (G6). |
| G4 | Licenses of all dependencies checked and listed (also gpui-ce) | Beta | Done | `cargo deny` license check (S8); policy, exceptions and the generated list in `docs/DEPENDENCIES.md`. Next: a third-party notice file in the release binaries (DEPENDENCIES §4 L1). |
| G5 | Privacy statement: no telemetry by default; data stays with the user | Beta | Done | `docs/PRIVACY.md` |
| G6 | Public repository and issue tracker | 1.0 | Open | The repo is private now. |

## 15. Owner decisions and open questions

Decisions of 2026-10-07:

| # | Question | Decision |
|---|---|---|
| PR-Q1 | Is mobile part of 1.0? | **Yes. 1.0 covers all platforms:** Windows, macOS, Linux, web, iOS, Android. Plan with the assumption that gpui mobile backends exist. |
| PR-Q2 | Does 1.0 need localization (U8) and sound (U11)? | **Yes, both.** |
| PR-Q3 | What is the support policy? | **Rolling support with no guaranteed window.** The promise is that every released runtime stays available (C10), so an old app keeps running. |
| PR-Q4 | Is the list complete? | Good so far. Add items when work finds them. |
| — | License (G1) | MIT OR Apache-2.0, as in the Rust ecosystem. |
| — | Name (G2), public repo (G6), Linux and macOS window (P2, P3) | At the end, before 1.0. |
| — | CI runners online (Q2), npm packages and release tests (D1, D2) | Near Beta. |
| — | VS Code Marketplace (T2) | At 1.0. |
| — | Structured storage (H4) | `store.sql`: a SQL store for each app. |
| — | Project key (S6, UP-Q4) | An offline root key held by the owner, and a release key for CI that the root key signs and can revoke. |
| — | `apply = "live"` (UP-Q2) | After 1.0. |
| — | Update feeds (UP-Q3) | The author's URL and the registries that the user adds. No Plinth-run service. |
| — | App tests (T3), 2026-10-08 | Written in Plinth TS: `*.test.tsx` files with `plinth:test`, run by `plinth test` and `npm test`. Not a JavaScript package. |
| — | Linux and macOS work, 2026-10-08 | Agents on Linux and macOS machines do it later. Until then, each item is tracked in `docs/PLATFORM-TODO.md`. |
| PR-Q5 | Does 1.0 need sync (H6)? | **No.** Sync comes after 1.0. Sync means: one user has the same app on two devices (for example a phone and a desktop), and the app data is the same on both. A change on one device goes to the other. Owner decision 7 says the data stays on the user's device or the user's storage, so sync goes through storage that the user owns (a folder that a sync service like OneDrive or iCloud copies, or a user-owned server), never a Plinth server. `docs/STORAGE.md` §6 has the design draft. |

## 16. Summary

Count on 2026-10-08, by gate and status. Run `bash scripts/production-status.sh` for the current count, and `bash scripts/production-status.sh --open Beta` for the list of open Beta items.

| Gate | Done | Partial | Open or Decide |
|---|---|---|---|
| Preview | 26 | 0 | 0 |
| Beta | 27 | 12 | 13 |
| 1.0 | 1 | 2 | 20 |
| Later (after 1.0) | 0 | 0 | 1 |

**Work for Linux and macOS machines** (the GUI window, the desktop tests, signing, Safari, CI) is collected in `docs/PLATFORM-TODO.md`. The owner will run agents on those machines later.

**The Preview level is complete** for the documented platforms (Windows and the web, `docs/LIMITS.md`). Preview is only an internal mark until the repository is public (G6).

**The next level is Beta.** Start with the items that the owner and the code do not block:

1. T7, the core part: optimize the 1.13 core with `wasm-opt -Oz` when it is built.
2. R6: performance budgets in the gate (done).
3. L9: source locations in runtime errors (done; release builds have no trap place).
4. R4, Q4: wasmi in all suites and in CI (done).
5. H4: the `store.sql` design and the first slice.
6. U5, U6, U7: IME on the desktop, pointer gaps, system settings.
