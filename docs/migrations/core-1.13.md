# Core 1.13

Status: not shipped yet. Core 1.12 is the newest shipped core (`rt_abi::SHIPPED_MINOR`). Items can still be added to 1.13 until it ships.

Core 1.13 adds the SQL store (`store.sql`, `docs/SQL.md`). Nothing is removed or changed. Every app that runs on core 1.12 runs on core 1.13 with the same behavior.

## 1. What is new

| Item | Kind | Notes |
|---|---|---|
| `sql_call` | Core function | Starts one `store.sql` call. The result comes back through the same completion path as `plinth:files` (`net_result_*`). |
| `plinth:app/sql` with `run(kind, statement, params) -> u32` | Host interface (`wit/plinth/app.wit`) | Kind 0 is `exec`, 1 is `query`, 2 is `batch`, 3 is `migrate`. `params` and the result are JSON text. |
| `sql` in `plinth:store` | Plinth TS module export (`std/store.d.ts`) | `sql.exec`, `sql.query<T>`, `sql.batch`, `sql.migrate`. |
| `store.sql` | Capability, risk Low | Declare it in `plinth.toml` with a rationale. |
| `PL2026` | Compiler warning | A SQL statement that is not a string literal or a `const`. |

## 2. App authors

You do not have to change an app.

- **An app that does not use `sql`** still declares core 1.12 as the lowest core it needs (`plinth-core` section, SPEC §10.5). It keeps running on hosts that have only core 1.12.
- **An app that uses `sql`** declares core 1.13. A host that has only older cores cannot run it. The host tells the user which core to install. Before you publish such an app, make sure that your users have a host with core 1.13.
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

## 3. Host authors

A host that uses core 1.13 must supply `plinth:app/sql` `run`, or the core does not link (`docs/COMPATIBILITY.md` §3).

- A host with no SQL engine answers each call with `[false, 0, "", "denied:unsupported"]`. It must not leave the import out.
- A host with a SQL engine follows `docs/SQL.md` §4: the authorizer, the time limit, the quota, the result limit, and the error texts of `crates/plinth-runner-wasmtime/src/sql.rs`. `crates/plinth-compiler/tests/sql_cases.txt` has the shared cases; the desktop and the web host both pass them.
- The host checks the capability `store.sql` before the call, as for `plinth:files`.
- The hosts in this repo do all of this: the desktop host (wasmtime and wasmi), the web host, the web Hub and the folder form of a web export. A single-file web export answers `denied:unsupported`.

## 4. Users

Nothing changes for users. An app that uses `store.sql` shows the capability in its details with its rationale. Its risk is Low, so the Hub does not ask at install.
