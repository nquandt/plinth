# Changelog

This file lists the changes in each Plinth release. Add an entry to "Unreleased" when a change merges that an app author or a user can see. A release moves "Unreleased" under its version number. `docs/PRODUCTION.md` tells what each level (Preview, Beta, 1.0) promises.

## Unreleased

No public release yet. The first release is a Preview for Windows and the web. `docs/LIMITS.md` lists what Preview cannot do.

### Added

- License: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- `docs/PRODUCTION.md`: the production readiness checklist.
- `docs/LIMITS.md`: the known limits.
- `SECURITY.md` and `docs/THREAT-MODEL.md`.
- `docs/COMPATIBILITY.md`: the public contracts, their version rules and the deprecation policy.
- `docs/PRIVACY.md`, `CODE_OF_CONDUCT.md`, and a pull request guide in `CONTRIBUTING.md`.
- `docs/host-apis.md` "Support on each host": how each host API works on the desktop (wasmtime and wasmi) and the web, and the test that covers it on each host.
- `docs/guides/`: guides for app authors (storage, network, updates, publishing, accessibility, performance), each with a checked example.
- Desktop keyboard (PRODUCTION U4): Tab and Shift+Tab move the focus (before, they did nothing), with a focus ring in the accent color on every control. Arrow keys choose in a `Picker` and a `Tabs`, step a `NumberField`, and move in an open `Menu` and `DatePicker`; Home and End move a `Slider`. A `Sheet` and a `Dialog` keep the focus while they are open and close on Escape. Menus and popovers give the focus back to their trigger.
- Desktop accessibility (PRODUCTION U3): the navigation, the back button, the toolbar overflow, the step buttons, the `Badge`, the `Empty` texts and the dialog buttons are in the AccessKit tree with a role and a name; headings have a level; a `Slider`, a `NumberField` and a `Progress` have their range; a short `Picker` is a radio group, a long one a list box; text fields have a named group.
- Web: a `Toggle` has the role `switch`, as on the desktop.
- `plinth:notify` (core 1.12, capability `notify`, Medium risk): `show({ title, body?, tag? })` shows a system notification and resolves with `"shown"` or `"unsupported"`; `cancel(tag)` removes it. The desktop host shows a toast on Windows. Linux, macOS and the web answer "unsupported" for now. Example: `examples/notify-demo`.
- Desktop self-update: an update that adds a Medium or High capability shows the consent window for the new capabilities before it runs. If the user cancels, the previous version keeps running, and the host does not ask again for that version.
- Desktop self-update: `apply = "prompt"` shows "An update is ready. Restart now?" with "Restart" and "Later" in the app window when an update is ready.
- The `plinth-host` executable now checks for updates when it runs an exported app, the same as the `plinth` executable.
- Dependency audit: `cargo deny` (`deny.toml`: advisories, licenses, bans, sources) and `npm audit` run in `scripts/ci-local.sh full`, in the new `deny` mode, and in Linux CI. `docs/DEPENDENCIES.md` gives the license policy, the exceptions and the list of licenses in use (`node scripts/gen-license-list.mjs --write`).
- Web host: `plinth:notify` shows notifications with the browser's `Notification` API. The page shows them for the app's sandboxed frame. A tag replaces an earlier notification, and `cancel` closes it. If the browser blocks notifications for the page, `show` rejects with `denied:refused`.
- Regular expressions: the `String` methods `match`, `matchAll`, `replace`, `replaceAll`, `split` and `search` take a `RegExp`, with the JavaScript rules for `g`, `lastIndex`, empty matches and the replacement patterns `$&`, `$1`..`$99`, `$<name>`, `$$`, `` $` `` and `$'` (core 1.12). `replace` and `replaceAll` also take a replacement function. `docs/language.md` "Regular expressions" lists the differences from JavaScript.
- `plinth build --release` (also with `--target web`, and `plinth native <dir> --release`): makes the app module about 14 % smaller with Binaryen `wasm-opt -Oz`, which is built into `plinth`. The output is deterministic, and `plinth` checks that the optimized module keeps its imports and its `plinth-core` section and links into the core. `docs/guides/performance.md` §8 has the measurements.
- `store.sql`, step 1 (`docs/SQL.md` §7, PRODUCTION H4): the desktop runner has a SQLite engine for each app (`<app id>+sql/store.sqlite`, next to the private space so that `plinth:files` cannot reach it; `rusqlite` with the bundled SQLite and FTS5). It has `exec`, `query`, `batch` and `migrate` on a worker thread, an authorizer (no `ATTACH`, no extensions, only safe `PRAGMA`s), a 2 s statement limit, a 256 MiB quota and a 16 MiB result limit. Apps cannot call it yet: the API comes with core 1.13 (step 2).
- `store.sql`, step 2 (core 1.13, capability `store.sql`, Low risk): `import { sql } from "plinth:store"` gives `sql.exec(statement, params?)`, `sql.query<T>(statement, params?)`, `sql.batch(steps)` and `sql.migrate(steps)`, each a `Promise`. `query<T>` decodes the rows into `T` (a `boolean` field reads 0 and 1); a row that does not match `T` rejects the promise. A SQL error rejects with `Error("sql: ...")`. The compiler gives warning `PL2026` for a statement that is not a string literal or a `const`. Desktop (wasmtime and wasmi) only; the web host answers `denied:unsupported` until step 3.
- `store.sql`, step 3 (web host): the page runs the official SQLite Wasm build (3.53.4, `web/vendor/sqlite-wasm/`) in its own Worker with OPFS (`opfs-sahpool`), one database for each app id. The app's calls go over the frame bridge (`sql-run`). The authorizer, the limits and the error texts are the same as on the desktop; `docs/SQL.md` §9 lists the differences (for example, without OPFS the database is in memory). The web App Hub and `web/index.html` have it; a web export does not have it yet.
- Core 1.13 starts (not shipped): it adds `sql_call`. An app that uses `store.sql` needs core 1.13; other apps still run on core 1.12.

### Changed

- A runtime error names the source file and line (PRODUCTION L9). "This app stopped" gives the reason, then the place of the innermost app function: `array index out of bounds (app/main.tsx:12)`, on the desktop (wasmtime) and on the web. An uncaught exception names the place of its `throw`: `Uncaught Error: boom (app/main.tsx:12)`. The compiler writes a small source map (the custom section `plinth-srcmap`, 3 % to 7 % of `app.wasm`); `plinth build --release` removes it, so a release build gives the plain trap reason. Packages from an earlier compiler give the plain reasons, as before.

- Core 1.12.0 is shipped and frozen. New core functions go into 1.13. The shipped file is in `cores/shipped/`.

- Web host: each app runs in its own Worker (a step to the web time limit). A page that embeds Plinth with a Content-Security-Policy must also allow `worker-src blob:`.

- Package format compatibility (`docs/COMPATIBILITY.md` §4): a package with a newer `plinth-profile` (`plinth/2` or higher) gives "This app needs a newer version of Plinth (package format plinth/N)" on desktop and on the web, not an "invalid package" error. A package entry that the host does not know is kept and signed, but never run and never served as an asset (before, the package was refused). `manifest.toml` can have unknown fields at every level, also in `[[capabilities]]` and `[updates]`, and an unknown `[updates]` `channel`, `check` or `apply` value maps to the default. `plinth.toml` stays strict: an unknown field or value there is still an error.

- Regular expressions: literals, `new RegExp`, `test`, `exec`, `lastIndex`, the flags `g i m s`, with a linear-time matcher in the core (core 1.12). `docs/language.md` "Regular expressions" lists the subset.

### Fixed

- `store.sql` on the desktop: the keys of a row are always in column order. Before, the order changed with the build (sorted, or in column order when another crate turned on the `preserve_order` feature of `serde_json`). A `query<Map<string, V>>` row now has the column order on every host.
- `core = "patch"` now keeps an app on patches of the core minor version that built it. Before, it used the lowest core the app needs, so a small app built with core 1.12 did not link on a host with only 1.12. The manifest records `core-built`.

### Security

- Web host: a time limit of 2 s for each call into an app, as on the desktop. An endless loop no longer freezes the page: the app's Worker ends, and the app shows "This app stopped" with "the app is not responding (time limit reached)".
- Web host: an app's memory is capped at 64 MiB, the same as the desktop host. A runaway allocation stops the app with "out of memory" instead of using the browser tab's memory.
- `plinth:net`: the `net.local` check now covers all non-public IPv4 and IPv6 ranges (for example `169.254.169.254`, `100.64.0.0/10`, `fd00::/8`, `::ffff:127.0.0.1`) and names such as `localhost.` and `*.localhost`, on both hosts. The desktop host also checks the addresses after DNS resolution, so a public name that resolves to a private address is denied without `net.local`.
- Package signatures: the new schema `plinth.signature/2` also covers `manifest.toml` (its raw bytes), `plinth-profile` and every other entry. Before, a person could change the manifest of a signed package (its `version`, `id`, capabilities, `[updates]` feed or `core` policy) and the signature still verified. Hosts and the web Hub refuse a package signed with `plinth.signature/1`: sign it again with `plinth sign`.
