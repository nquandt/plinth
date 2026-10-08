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
| C3 | Ship core 1.12 and freeze it (no more additions to a shipped minor) | Beta | Open | HANDOFF §6: 1.12 is not shipped. Ship it with the first Beta release. |
| C4 | `.plnt` container and manifest have a format version and a compatibility rule | Beta | Done | `docs/COMPATIBILITY.md` §4: a newer `plinth-profile` gives "This app needs a newer version of Plinth"; unknown entries are kept (signed, never run, never served); the manifest tolerates unknown fields and enum values at every level; `plinth.toml` stays strict. Tests in `plinth-package`, `plinth-hub` and `web/test/run-hub-integrity.mjs`. Frozen old packages: C8. |
| C5 | Host interfaces (`wit/plinth/app.wit`) have a version and an append-only rule | Beta | Done | `docs/COMPATIBILITY.md` §3; `plinth-link/tests/wit_append_only.rs` with `wit/plinth/app.wit.snapshot` |
| C6 | Plinth TS language version: a list of the accepted syntax and the deviations from ECMAScript | Beta | Partial | `docs/COMPATIBILITY.md` §5 (rule: a source that compiles stays compiling). Next: freeze the 1.0 subset in `docs/language.md` at Beta. |
| C7 | Deprecation policy: how a feature is marked, how long it stays, what diagnostic the compiler gives | Beta | Done | `docs/COMPATIBILITY.md` §6. The first deprecation adds its `PLW` golden test. |
| C8 | Compatibility test suite: `.plnt` files built by each released version run on the newest host | 1.0 | Open | Keep the released packages of `examples/` in the repo and run them in the gate. |
| C9 | Support policy: rolling support, no guaranteed window; every released runtime core stays available | 1.0 | Done | §15 PR-Q3 (owner, 2026-10-07) |
| C10 | Every released core and host stays available for download, so an old app always finds its core | 1.0 | Open | A permanent core archive (registry or release assets) and a test that installs an old core from it. |

## 3. Language and compiler

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| L1 | Core TypeScript subset: unions, narrowing, generics, classes, `Map`/`Set`, tuples, destructuring, JSON, `try`/`catch`, `async`/`await`, Promise API | Preview | Done | HANDOFF §2 "Compiler", `tests/language*` (171 cases) |
| L2 | Every rejected feature gives a diagnostic with a golden test | Preview | Done | `plinth-compiler/tests/golden.rs` (SPEC M3 exit) |
| L3 | The compiler never panics or overflows its stack on any input | Preview | Done | `tests/robustness.rs`, `PL1000`, `stacker` |
| L4 | Regular expressions (`RegExp`) | Beta | Partial | Linear-time matcher in the core (`plinth-rt/src/regex.rs`), literals, `new RegExp`, `test`, `exec`, collectable objects (`tests/lang.rs` `regexp_*`), `docs/language.md` "Regular expressions". Next: `String.match/matchAll/replace/split/search` with a `RegExp`. |
| L5 | Locale-aware text: number, date and plural formatting (`plinth:locale`) | Beta | Open | `docs/I18N.md` §3.3 (steps 1–2). |
| L6 | Long expression chains (more than 1000 terms) | Beta | Open | GAPS R3. |
| L7 | Smaller async code (promise machinery in the core) | 1.0 | Open | GAPS "Async code size". |
| L8 | Libraries: share Plinth TS code between apps (`hub:<publisher>/<lib>`, `plinth.lock`) | 1.0 | Open | SPEC §12 "Libraries". Without them, each app copies code. |
| L9 | Runtime errors name the source file and the line (source maps for stack traces) | Beta | Open | Today "This app stopped" gives a reason, not a source location. |

## 4. Runtime and engines

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| R1 | GC and allocator have no known leaks; memory stays level under random use | Preview | Done | `tests/monkey.rs`, GAPS "Robustness" |
| R2 | Desktop limits: time limit, memory cap, stack limit stop only the app, with a plain reason | Preview | Done | `tests/errors.rs` "Limits" |
| R3 | Web limits: a time limit (Worker and watchdog) and a memory cap | Beta | Partial | Memory cap done (`capCoreMemory`, `web/test/run-stop.mjs`). Time limit: design in GAPS "R1/R2 design"; next is the Worker split (step 1), then the watchdog. |
| R4 | wasmi engine passes all suites (monkey, async, files, hub, net) and runs in CI | Beta | Partial | HANDOFF §10 item 1 (a)–(d) |
| R5 | Engine-free host state crate (no wasmtime dependency for iOS) | 1.0 | Open | HANDOFF §10 item 1 (e). Needed for iOS (PR-Q1). |
| R6 | Performance budgets in the gate: start time, frame cost, memory per app, `.plnt` size | Beta | Partial | `frame_cost.rs`, counter size guard in `tests/e2e.rs`. Next: write the budgets in a doc and fail the gate when an example exceeds them. |
| R7 | Nested flex layout is linear (gpui-ce fix) | Beta | Open | GAPS F1; HANDOFF §10 item 3 |

## 5. UI and accessibility

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| U1 | Level 1 controls and Level 2 primitives, same tree on desktop and web | Preview | Done | `docs/ui.md`, UI API 1.13 |
| U2 | Web accessibility: axe checks pass on all examples | Preview | Done | `web/test/run-a11y.mjs` |
| U3 | Desktop accessibility: AccessKit tree with names and roles for every control | Beta | Partial | `plinth-shoot/tests/a11y_controls.rs` walks every Level 1 control and Level 2 primitive (role, name, value, state, disabled, expanded, selected; each node with a focus or click action has a name). Same semantics as the web except: a `Picker` with more than 4 options (desktop list box, web `<select>`), `DatePicker` (desktop button and grid dialog, web native input), the selected `Row` and the current screen (desktop `selected` / no current state, web `aria-current`), `Scroll` (a tab stop only on the web), a text field's name (desktop: on the group around the input, a gpui-ce limit). Next: a manual check with Narrator, VoiceOver and Orca; the text input name in gpui-ce. |
| U4 | Keyboard: every action is possible without a pointer; visible focus | Beta | Partial | Desktop: `plinth-shoot/tests/a11y_controls.rs` (Tab and Shift+Tab order, Enter and Space, typing, focus ring in a screenshot, Escape on menus, popovers, Sheet and Dialog, focus trap in modals, arrow keys in Picker, Tabs, Slider, NumberField, DatePicker, Menu). Web: native elements and the menu and tabs key handlers. Next: a keyboard-only test of each example on both hosts, and the same fixture in `run-a11y.mjs`. |
| U5 | Text input: IME composition, on desktop too | Beta | Open | The web uses native inputs. Check the gpui-ce text field with a CJK IME. |
| U6 | Pointer: secondary button or `onContextMenu`, drag and drop, modal overlay that blocks pointer events | Beta | Open | GAPS G4, 7G-6, 7G-7; HANDOFF §10 item 6 |
| U7 | System settings: dark mode, high contrast, text scale, reduced motion | Beta | Partial | Theme tokens exist. Next: follow each OS setting on both hosts, with tests. |
| U8 | Localization: app strings in more than one language, the user's locale | 1.0 | Open | Design draft: `docs/I18N.md` (catalogs, `t()`, host formatting with ICU4X and `Intl`). Next: answer I18N-Q1..Q5, then steps 1–4. |
| U9 | Right-to-left layout | 1.0 | Open | `docs/I18N.md` §3.5. Risk: bidirectional text in gpui-ce (I18N-Q1). |
| U10 | Motion: host-side transitions and springs | 1.0 | Open | UI-ADVANCED U2; HANDOFF §10 items 4, 7 |
| U11 | Sound | 1.0 | Open | GAPS G7. 1.0 needs it (PR-Q2). |

## 6. Host APIs

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| H1 | `time`, `store` (kv), `clipboard`, `dialog`, `net`, `files`, `error.report` on all hosts | Preview | Done | `docs/host-apis.md` |
| H2 | Denied calls never trap; distinct denial reasons | Preview | Done | SPEC §11 |
| H3 | Notifications (`notify`) | Beta | Partial | Step 1 done: WIT `notify` (`show`, `cancel`), core 1.12 functions, the `notify` capability and policy, a `NotifyBackend` with a test backend on wasmtime and wasmi (`tests/notify.rs`). Next: the `plinth:notify` module in the compiler, a native toast on the desktop, the web host through the parent page. |
| H4 | Structured storage (`store.sql` or a documented pattern on files) | Beta | Decide | STORAGE ST-Q1, VALIDATION V2 |
| H5 | Storage: `vault` space, a folder as a space, `watch` | 1.0 | Open | `docs/STORAGE.md` §6 |
| H6 | Sync between devices of one user | Later | Open | STORAGE §6. Not in 1.0 (PR-Q5). |
| H7 | Each host API has a documented behavior on each host, with the same test on each host | Beta | Partial | `docs/host-apis.md` "Support on each host" (behavior and tests: desktop wasmtime, desktop wasmi, web). Next: tests for the "no test" cells: `clipboard` on all three hosts; `dialog`, `net`, `files` and `hub` on wasmi (with Q4); `dialog` on the web. |

## 7. Security

Production apps run code from many publishers. This area must be complete before 1.0, and a review by a third party is necessary.

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| S1 | Default deny; capabilities proved from imports; user consent | Preview | Done | SPEC §11, consent window |
| S2 | Every web app in a sandboxed iframe with an opaque origin | Preview | Done | SPEC §10.3 |
| S3 | `net` blocks private network ranges unless `net.local` | Preview | Done | All non-public IPv4/IPv6 ranges and embedded IPv4 on both hosts (`policy.rs` `is_private_ip`, `plinth-web.js` `isPrivateNetHost`, `web/test/run-net-private.mjs`); the desktop also checks the address after DNS resolution (`net.rs` `PublicOnlyResolver`, `tests/net.rs`). The web cannot check DNS results (THREAT-MODEL §6 item 1). |
| S4 | Packages and update feeds are signed (Ed25519); no downgrade | Preview | Done | `plinth-package::feed`, desktop updater |
| S5 | Re-consent when an update asks for new capabilities | Beta | Done | `plinth-host-desktop::updater` (`update_consent`, `prepare_start`), the Hub (`docs/HUB.md` §7.3 step 3); UPDATES §6 |
| S6 | Project key that signs cores and the Hub; key rotation and revocation | Beta | Decide | HANDOFF §0.2 item 3; HANDOFF §10 item 12 |
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
| D4 | Update `prompt` dialog and `onUpgrade(fromVersion)` | Beta | Partial | The desktop restart prompt is done (UPDATES §5). Next: `onUpgrade(fromVersion)` (needs a core change; UPDATES §9.1). |
| D5 | Web updates (service worker cache) | Beta | Open | UPDATES §9.1 |
| D6 | A public registry and the Hub app on desktop and web | Beta | Partial | `docs/HUB.md`, `docs/REGISTRY.md`, `docs/web-hub.md`. Next: host a first registry. |
| D7 | Update policy decisions (UP-Q1..Q6) | Beta | Decide | `docs/UPDATES.md` |

## 10. Developer tools

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| T1 | `plinth new`, `dev` (hot reload), `check`, `build`, `run`, `native` | Preview | Done | SPEC §13 |
| T2 | VS Code extension: diagnostics, completion | 1.0 | Partial | `editors/vscode`. Next: publish it to the Marketplace. |
| T3 | A test kit for app authors: render a screen, send events, check the tree | Beta | Open | The internal harness in `tests/apps.rs` can be the base. |
| T4 | A debugger or an inspector: the UI tree, signals, host calls | Beta | Open | |
| T5 | A profiler view: frame times, GC, memory (`PLINTH_TRACE_RENDER` as a start) | 1.0 | Open | |
| T6 | `plinth publish` to a registry | Beta | Partial | SPEC §12, `plinth hub` |
| T7 | `plinth build --release` optimizes the app module with Binaryen `wasm-opt` (and the release core) | Beta | Open | SPEC §5.1 step 10. Measure size and speed before and after; keep the result deterministic. |

## 11. Documentation

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| X1 | Getting started from zero to a running app | Preview | Done | `docs/getting-started.md` |
| X2 | Reference: language, UI, host APIs | Preview | Done | `docs/language.md`, `docs/ui.md`, `docs/host-apis.md` |
| X3 | A public docs website with search | Beta | Open | |
| X4 | Guides: storage, network, updates, publishing, accessibility, performance | Beta | Done | `docs/guides/` (index `docs/guides/README.md`); each example checked with `plinth check`. |
| X5 | A migration guide for each breaking change and each core minor | Beta | Open | |
| X6 | `CHANGELOG.md` | Preview | Done | `CHANGELOG.md`. Add an entry for each change that authors or users see. |
| X7 | Known limits page (what Plinth cannot do yet) | Preview | Done | `docs/LIMITS.md`. Update it when a limit goes away. |

## 12. Quality gates and CI

| # | Item | Gate | Status | Evidence / Next |
|---|---|---|---|---|
| Q1 | Local gate: `scripts/ci-local.sh full --shoot`, `run-a11y.mjs` | Preview | Done | HANDOFF §3 |
| Q2 | CI runs on each push (self-hosted Windows and Linux runners online) | Beta | Partial | HANDOFF §0.2 item 1 (owner steps open) |
| Q3 | CI on macOS for each release, not only weekly | Beta | Open | |
| Q4 | A wasmi job in CI | Beta | Open | HANDOFF §10 item 1 (d) |
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
| PR-Q5 | Does 1.0 need sync (H6)? | **No.** Sync comes after 1.0. Sync means: one user has the same app on two devices (for example a phone and a desktop), and the app data is the same on both. A change on one device goes to the other. Owner decision 7 says the data stays on the user's device or the user's storage, so sync goes through storage that the user owns (a folder that a sync service like OneDrive or iCloud copies, or a user-owned server), never a Plinth server. `docs/STORAGE.md` §6 has the design draft. |

## 16. Summary

Count on 2026-10-07, by gate and status. Run `bash scripts/production-status.sh` for the current count, and `bash scripts/production-status.sh --open Beta` for the list of open Beta items.

| Gate | Done | Partial | Open or Decide |
|---|---|---|---|
| Preview | 26 | 0 | 0 |
| Beta | 11 | 17 | 24 |
| 1.0 | 1 | 1 | 21 |
| Later (after 1.0) | 0 | 0 | 1 |

**The Preview level is complete** for the documented platforms (Windows and the web, `docs/LIMITS.md`). Preview is only an internal mark until the repository is public (G6).

**The next level is Beta.** Start with the items that the owner and the code do not block:

1. L4 and H3: finish `RegExp` and notifications (on branches).
2. C3: ship core 1.12 and freeze it, after L4 and H3 merge.
3. R3: the web time limit (the Worker split is on a branch).
4. U3, U4: desktop accessibility and keyboard tests (on a branch).
5. T7, R6, L9: `wasm-opt`, performance budgets, source locations in errors.
