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
