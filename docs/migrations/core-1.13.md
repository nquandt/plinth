# Core 1.13

Status: not shipped yet. Core 1.12 is the newest shipped core (`rt_abi::SHIPPED_MINOR`). Items can still be added to 1.13 until it ships.

Core 1.13 adds the SQL store (`store.sql`, `docs/SQL.md`), a hash table for `Map` and `Set`, and the module `plinth:locale` (`docs/I18N.md` §4 step 1). Nothing is removed or changed. Every app that runs on core 1.12 runs on core 1.13 with the same behavior.

## 1. What is new

| Item | Kind | Notes |
|---|---|---|
| `sql_call` | Core function | Starts one `store.sql` call. The result comes back through the same completion path as `plinth:files` (`net_result_*`). |
| `plinth:app/sql` with `run(kind, statement, params) -> u32` | Host interface (`wit/plinth/app.wit`) | Kind 0 is `exec`, 1 is `query`, 2 is `batch`, 3 is `migrate`. `params` and the result are JSON text. |
| `sql` in `plinth:store` | Plinth TS module export (`std/store.d.ts`) | `sql.exec`, `sql.query<T>`, `sql.batch`, `sql.migrate`. |
| `store.sql` | Capability, risk Low | Declare it in `plinth.toml` with a rationale. |
| `PL2026` | Compiler warning | A SQL statement that is not a string literal or a `const`. |
| `kv_find_str`, `kv_find_f64`, `kv_find_i32`, `kv_add`, `kv_remove`, `kv_clear`, `kv_size`, `kv_live`, `kv_live_copy`, `kv_iter` | Core functions (`crates/plinth-rt/src/kv.rs`) | A hash table for `Map` and `Set`. Before, each `get`, `set`, `has` and `delete` searched the keys one by one: 10,000 string keys took 1,558 ms, and now take 13 ms. A loop over a map visits each entry one time, also when the loop body adds or deletes keys. |
| `locale_tag`, `locale_direction` | Core functions | `locale()` and `direction()` of `plinth:locale`. Each read is reactive: the locale event makes it run again. |
| `plinth:app/locale` with `tag() -> string` and `is-rtl() -> bool` | Host interface (`wit/plinth/app.wit`) | Synchronous. No capability. `tag` is a BCP 47 tag and is never empty. |
| `lifecycle` event, kind 4 (locale changed) | Event (SPEC §8.4, `plinth_protocol::lifecycle_kind::LOCALE`) | No payload. The core reads the locale again. A core older than 1.13 ignores it. |
| `locale`, `direction` and the type `Direction` in `plinth:locale` | Plinth TS module (`std/locale.d.ts`) | `locale(): string`, `direction(): "ltr" \| "rtl"`. |
| Merge sort for `Array.prototype.sort` | Compiler output | No core function. A sort of 10,000 numbers took 560 ms, and now takes 5 ms. The sort stays stable, with the same order. |

## 2. App authors

You do not have to change an app.

- **An app that uses neither `sql` nor `Map` nor `Set`** still declares core 1.12 as the lowest core it needs (`plinth-core` section, SPEC §10.5). It keeps running on hosts that have only core 1.12.
- **An app that uses `Map` or `Set`** declares core 1.13 when you build it with the new compiler, because the new `Map` code uses the 1.13 functions. The same app, built with an older compiler, still declares the older core and keeps running.
- **An app that uses `plinth:locale`** declares core 1.13. An app that does not import it does not need core 1.13 because of it.
- **An app that uses `sql`** declares core 1.13. A host that has only older cores cannot run these apps. The host tells the user which core to install. Before you publish such an app, make sure that your users have a host with core 1.13.
- **The `core` policy** (`plinth.toml`, SPEC §10.5):
  - `follow` (the default): the app gets core 1.13 when the host has it. No action.
  - `patch`: the app stays on the minor version it was built with (`core-built`). Build it again with the new CLI to move it to 1.13.
  - `pinned`: the app stays on the exact core build. Build it again to move it.

To use the SQL store:

1. Add the capability to `plinth.toml`:

   ```toml
   [[capabilities]]
   name = "store.sql"
   rationale = "Save your notes on this device and search them."
   ```

2. Import `sql` from `plinth:store` and make the schema with `sql.migrate`. `docs/guides/storage.md` §3.1 has an example.
3. If the app keeps a list in `kv` today, move the data one time on the first run. `examples/notes/app/storage.ts` shows how: it reads the old `kv` value, writes the rows in one `sql.batch`, and then removes the `kv` key.
4. Keep `store.kv` in `plinth.toml` while the app moves old data. Its rationale tells the user why.

To use the locale, import `locale` and `direction` from `plinth:locale` (`docs/host-apis.md` "plinth:locale"). No capability is necessary. Read them in the UI, in a `computed` value or in an `effect`, so that the app follows a change of the locale.

## 3. Host authors

A host that uses core 1.13 must supply `plinth:app/sql` `run` and `plinth:app/locale` `tag` and `is-rtl`, or the core does not link (`docs/COMPATIBILITY.md` §3).

For `plinth:app/locale`:

- `tag` returns a BCP 47 tag, never an empty string (`en-US` when the host finds no locale). Normalize a POSIX name: `en_US.UTF-8` gives `en-US`.
- `is-rtl` follows the script subtag, else the language subtag, with the lists of `crates/plinth-runner-wasmtime/src/locale.rs` (the web host has the same lists in `web/plinth-web.js`).
- When the locale changes while the app runs, send a `lifecycle` event with kind 4. Send it only after the new value is ready for `tag`. An app with a core older than 1.13 ignores it.
- The web host sets `lang` and `dir` on the root element of the app.

- A host with no SQL engine answers each call with `[false, 0, "", "denied:unsupported"]`. It must not leave the import out.
- A host with a SQL engine follows `docs/SQL.md` §4: the authorizer, the time limit, the quota, the result limit, and the error texts of `crates/plinth-runner-wasmtime/src/sql.rs`. `crates/plinth-compiler/tests/sql_cases.txt` has the shared cases; the desktop and the web host both pass them.
- The host checks the capability `store.sql` before the call, as for `plinth:files`.
- The hosts in this repo do all of this: the desktop host (wasmtime and wasmi), the web host, the web Hub and the folder form of a web export. A single-file web export answers `denied:unsupported`.

## 4. Users

Nothing changes for users. An app that uses `store.sql` shows the capability in its details with its rationale. Its risk is Low, so the Hub does not ask at install.
