# History of the work

> This file keeps the session logs that were in `HANDOFF.md`. Newest last. It uses ASD-STE100 Simplified Technical English, as `SPEC.md` does.

## Session log 2026-10-07 (what changed and why)

- **Robustness:** compiler mutation test (fixed a `break` outside a loop, JSON of recursive types, deep nesting with `stacker` and `PL1000`); runner limit tests; plain stop reasons on both hosts; web host stopped state and banners; hostile op streams, hostile render trees and changed app modules (no bugs found; tests kept).
- **Language:** default values in patterns, object rest, optional props set to `undefined` for every prop kind, `onMount`.
- **Memory:** three core leaks found by the monkey test and fixed (allocator tail loss, first fit, uncounted array buffers).
- **Performance:** frame-cost survey; labelled form controls about two times faster (block layout; GAPS F1); async code size measured.
- **Platform decisions:** `docs/UPDATES.md` (three-layer updates), wasmi for iOS, core patch versions and the `core` policy.
- **CI:** self-hosted runner files and `ci.yml` (owner steps open, §0.2). Pushed to GitHub up to `b6e766e`.

## Session log 2026-10-07, second part (production checklist)

- **`docs/PRODUCTION.md`:** gates Preview, Beta, 1.0 and Later; owner decisions in §15 (1.0 = all platforms, localization and sound; rolling support; MIT OR Apache-2.0; no sync in 1.0). Preview is complete. New docs: `docs/LIMITS.md`, `CHANGELOG.md`, `SECURITY.md`, `docs/THREAT-MODEL.md`, `docs/COMPATIBILITY.md`, `docs/PRIVACY.md`, `docs/I18N.md` (design), `docs/DEPENDENCIES.md`, `docs/guides/`, `CODE_OF_CONDUCT.md`.
- **Security fixes:** `net.local` covered too few private ranges (both hosts; the desktop now also checks the address after DNS resolution); the package signature did not cover `manifest.toml` (now `plinth.signature/2`, which refuses `/1`).
- **Built:** RegExp (linear-time Pike VM in the core; literals, `new RegExp`, `test`, `exec`; collectable objects); notify step 1 (WIT, core, runner; no compiler module yet); updater re-consent and the restart prompt; package format compatibility; a WIT append-only snapshot test (`wit/plinth/app.wit.snapshot`, bless with `PLINTH_BLESS=1`); the web 64 MiB memory cap and a Worker for each web app; `cargo deny` and `npm audit` in the gate; desktop keyboard and accessibility for every control (`plinth-shoot/tests/a11y_controls.rs`).
- **Flaky tests fixed:** six test files named temp folders from the clock only (100 ns steps on Windows); they now add an atomic counter.
- **Lesson (agents):** seven parallel agents with cold worktree builds overloaded the PC for hours. Run at most one or two build agents; tell an agent to run only the tests of the crates it changes, then merge into `master` and run the full gate there (the cache of `master` is warm). Give an agent one bounded step; a large task makes it stop with no code.

## Session log 2026-10-08 (store.sql, core 1.13, UI, test kit)

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

## Session log 2026-10-08, evening (D4, C6, a compiler fix)

- **D4 `onUpgrade` (Done):** step C on the web host. The data version goes through `WorkerApp` (a `load` field and a `data-version-set` message), the Hub (`plinth-hub:dv:<id>`, removed with the app's kv data) and the stand-alone page (`plinth-data-version:<id>`). `web/test/run-upgrade.mjs` covers a reload through `WorkerApp` and the Hub storage. Docs: ui.md, the core 1.13 guide (the host rule), UPDATES §6 with the UP-Q5 interim rule (a downgrade keeps the higher version, no handler), SPEC §4.7, §8.5.
- **C6 (Done):** `docs/language.md` "Language version 1.0" freezes the subset and lists what a 1.x compiler can add.
- **Compiler fix:** a function with no return type that mixes `return;` and `return <value>` gave an invalid module (only when the value branch came first in the source). Now its type is `T | null` (`finish_ret`, `patch_returns`). `onUpgrade` accepts a handler of type `Promise<void> | null`.
- **`===` on `string | Shape`** compares two strings by content (`EqKind::UnionStr`: the type tag at offset 0 of each side, then `str_eq`); also `switch`, `includes`, `indexOf`.
- **GAPS R3 (Done):** a `??`/`?.`/`||` chain reuses one local per type (`chain_temp`); 60,000 links compile. The engine needs minutes to compile such a function in a debug build, so those tests only compile.
- **Narrowing of a function-local `let`** in code order, also in loops that assign it (`docs/language.md` has the 5 rules). Review found 5 unsound cases in the first agent versions (a closure that assigns, a narrowing from before a loop, a module-level `let` assigned by a function or a method): check each narrowing change against these cases.
- **Tests:** each diagnostic code has a golden case (`every_code_has_a_golden_case`; `PL3011` is reserved).
- **Lessons:** a Node test that calls `PlinthApp` directly does not test the browser path: every browser host goes through `WorkerApp`, so test host state across the Worker boundary and a reload. A Sonnet agent that waits on a background build gets stopped: tell it to run cargo in the foreground. After a merge, rebuild `plinth.exe` before a manual check (the gate's binary is older).

## 2026-10-27 (stabilize, personal registry, Mac handoff)

- **Dependencies:** `cargo deny` failed on new advisories. `wasmtime` 46.0.3 is now 48.0.7 (no code change), `zip` 2.4 is now 9.1 (`ZipFile::name()` returns a `Result`), `rustls` 0.23.46, `rustls-webpki` 0.103.16. The two old `ignore` entries in `deny.toml` are gone (`docs/DEPENDENCIES.md` A1).
- **Personal registry:** `scripts/registry-publish.sh`, `infra/registry/` (Caddy and `cloudflared` in Docker, for `plinth.femur.dev`), `infra/azure/provision.sh` (resource group `plinth-rg`, storage account `plinthreg36610` with a static website). Design and plans: `docs/REGISTRY-HOSTING.md`.
- **iOS:** found that gpui-ce has no iOS platform crate. `docs/MAC-HANDOFF.md` has the plan and `hosts/ios-shell/` has a `WKWebView` bridge (never built).
- **Repository:** merged `wt/*` branches deleted. `wt/lang` is not merged (old size-limit change; superseded by owner decision 6).
- **Gate (2026-10-27):** `bash scripts/ci-local.sh full` passes; `node web/test/run-a11y.mjs` passes on the second run (the first run failed at the known web Hub start flake, HANDOFF §7). The publish script, the Docker site (Caddy) and the Azure mirror were tested with a scratch app; the mirror was emptied after the test.

## 2026-10-28 (secrets, Access, uninstall, publish; first desktop test against the live registry)

- **Built and merged (gate passed, pushed):** `plinth:secrets` (Windows Credential Manager, web WebCrypto; `docs/SECRETS.md`), registry source headers and `plinth hub source set-token` (Cloudflare Access), uninstall with data, quotas and export (`docs/STORAGE.md` §7), the Hub Uninstall action, `plinth publish` (T6), R5 step 1 (`wasmtime` optional), and the WIT drift test.
- **Windows desktop test against `https://plinth.femur.dev/` (2026-10-28):** `plinth hub source add`, `set-token` (token in the Credential Manager), `search`, `install`, `plinth publish` of a new version, `update`, all worked through Access. With no token and with a wrong token, the error names Cloudflare Access and `set-token`. `plinth hub ui` opened the native Hub window and showed the installed app "from plinth-femur".
- **Findings:** (1) a stale test source (`local`, a folder in `target/`) broke `hub search` for all sources: one bad source fails the whole search. (2) An old Hub package in the library (old signature format, another key) could not run or update; the fix was `plinth hub remove dev.plinth.hub` and install again. (3) The native host trusts a Hub key only through `PLINTH_HUB_TRUSTED_KEYS`; a saved setting is being added. (4) The Hub key rotation is not supported yet.
- **Lessons:** a background command that ends with exit code 0 can hide a failed gate: read the log. Agent worktrees each hold a `target` folder of 8 GB or more; the disk was full (2 GB free). Remove merged worktrees after each merge.
- **WebKit and Firefox (2026-10-28, Windows):** installed with `npx playwright install webkit firefox`. `PLINTH_BROWSER=webkit|firefox|chromium` picks the engine of `web/test/run-secrets.mjs` and `web/test/run-uninstall.mjs` (the two tests that use a real browser). Both pass in all three engines. WebCrypto and IndexedDB, the base of `plinth:secrets`, work in WebKit, the engine of iOS. The Playwright WebKit build for Windows has no OPFS sync access handle, so the SQL databases are in memory there; the uninstall test accepts that for non-Chromium engines. This is a limit of the test build, not a result for iOS Safari: test OPFS on a real iPhone. The other web tests run in Node, not in a browser. `plinth hub search` now skips a failing source (finding 1 is fixed).

## 2026-10-28 (app themes)

- **Built:** per-app theme token values (`[theme]` in `plinth.toml`, `docs/ui.md` "Theming an app"). Owner decision: stay with tokens; limits are a feature; no raw colors. Data model, hex checks and the contrast guard in `crates/plinth-package/src/theme.rs`; the desktop host applies the merged tokens (`Tokens::themed_with`); the web host reads the manifest in `web/theme.js` and sets the `--pl-*` variables on the app root only. A golden file (`web/test/fixtures/theme-golden.json`) is checked by Rust and Node, so both hosts give the same colors. `examples/themed` shows it.
- **Decisions:** (1) A contrast pair is checked only when the app sets one of its two tokens, because the host default light teal accent with white text is 4.2:1 and would fail every override. (2) An override of `background` alone fails in the other appearance, because the host dark text is faint on a light background; the error says so. (3) On the web, an app with a `[theme]` fills the tokens that it does not set from the desktop palette (the web host palette differs a little), so both hosts match. (4) The host banners use `--pl-host-danger` (computed on the root) so an app theme cannot change them.
- **Not tested:** macOS, Linux, iOS, Android, WebKit and Firefox.

## 2026-10-28 (app theme guard, second pass)

- **Built:** the contrast guard checks every pair after the merge, in light and dark (`crates/plinth-package/src/theme.rs`, `web/theme.js`). New pairs: `text` on `selected` over `surface` and over `background` (4.5:1), toggle track on `surface` and knob on track (1.5:1). The host light accents that failed 4.5:1 with white `on-accent` are darker (teal `#0b857a`, orange `#bf5900`; the others already passed). Tests: every named accent at least 4.5:1 with its `on-accent` in light and dark, a background-only theme, a failing `selected`, a hidden track. The golden file has two new fail cases.
- **Decisions:** (1) This replaces decision (1) and (2) of the first pass: no pair is skipped, and the host values must work with the app values. (2) The toggle track and knob stay fixed host colors (not derived from tokens); the guard uses 1.5:1 because 3:1 would reject the host track. (3) The `style.css` default accent of the web host (not a themed app) is a separate palette that already has at least 5:1 with white; it is not changed.
- **Not tested:** macOS, Linux, iOS, Android, WebKit and Firefox.

## 2026-10-28 (kanban probe)

- **Built:** `examples/kanban`, an issue board made only from the public UI API, to find the limits of the UI model. Result and next UI work: `docs/KANBAN-FINDINGS.md`.

## 2026-10-28 (kanban and the monkey test)

- **Found:** `examples/kanban` failed the monkey test ("memory grew", then "out of memory" at 64 MiB). Four causes. (1) The test did not remove the `store.sql` database (it is beside the files folder, `database_dir`), so the demo issues of earlier runs came back and the board had thousands of cards. (2) The test did not wait for host requests, so a slow database loaded its data in the second half of the run. (3) The app: "Add 500 demo issues" had no limit, and two presses before the first one ended read the same count. (4) The runtime: the GC threshold was always 1 MiB, so every app kept up to 1 MiB of garbage as memory, and a run showed a slow rise that is not a leak (a GC stress run was flat).
- **Built:** the test removes the sql database and waits for host requests (`open_host_requests`) at the dialog step; it does not press controls with "demo issues" in the label (the app limits them; a cap of 1,000 issues is still 12,000 nodes). kanban: `DEMO_LIMIT`, one demo action at a time, a test. `plinth-rt`: the threshold is the live size of the heap between 256 KiB and 1 MiB; test `small_apps_collect_early` in `lang.rs`.
- **Result:** all apps pass the default monkey test. kanban with 2,000 and 6,000 steps stays flat (2.0 MiB).
- **Not tested:** the cost in time of the more frequent collections in a large app (big-list has a live heap above 1 MiB, so its threshold did not change); a real release build; macOS, Linux, iOS, Android.
