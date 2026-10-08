# SQL store design (draft)

This document is the design of `store.sql`: a SQL database for each app. It is checklist item H4 in `docs/PRODUCTION.md`. The owner decided on 2026-10-07 that Plinth gives apps a SQL store (`docs/STORAGE.md` ST-Q1).

Status: 2026-10-08. Steps 1 and 2 of §7 are done: apps call `store.sql` on the desktop host, on wasmtime and on wasmi (core 1.13). The web host does not have it yet (step 3); there each call rejects with `Error("denied:unsupported")`.

## 1. Goals

1. An app keeps structured data and queries it: filter, sort, join, count, full-text search. Most production apps need this (VALIDATION V2, the notes app).
2. The same app code works on every host, with the same results.
3. The data belongs to the user (owner decision 7): the database is a file in the app's private space (`docs/STORAGE.md` §2), on the device or in the user's storage.
4. One app cannot read the database of another app.
5. A query cannot freeze the app or the host, and a database cannot fill the disk.

## 2. Engine

SQLite on every host, so that the SQL dialect and the results are the same.

| Host | Engine | Where the file is |
|---|---|---|
| Desktop | SQLite through `rusqlite` with the bundled SQLite build, in the host, on a worker thread | `<app id>+sql/store.sqlite`, next to the app's private space (`sql::database_dir`). The folder is not in the private space, so `plinth:files` cannot read or change the database, and the database does not count toward the files quota. |
| Mobile | The same as the desktop (gpui hosts, Rust) | The same as the desktop, on the device |
| Web | The official SQLite Wasm build in a Worker of the **parent page** (the host shell), with the OPFS VFS (`opfs-sahpool`) | OPFS of the page origin, one file for each app id |

**Why the parent page on the web:** an app runs in a sandboxed frame with an opaque origin, and its code runs in a Worker of that frame (SPEC §10.3). An opaque origin has no storage, so OPFS is not available there. The parent page has an origin and storage. So the parent runs the database, and the app's calls go over the existing bridge, as `files` calls do (`web/frame-host.js`).

**Why not the core:** a SQLite build is large, and it would make every core bigger and slower to load. The host owns it, as it owns the renderer.

## 3. API (`plinth:store`)

All calls are asynchronous on every host, because the web host must send them to the parent page (SPEC §8.5: a call that can block returns a request and finishes through a completion).

```ts
import { sql } from "plinth:store";

type Note = { id: number; title: string; done: boolean };

await sql.migrate([
  "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)",
  "CREATE INDEX notes_done ON notes (done)",
]);

const r = await sql.exec("INSERT INTO notes (title) VALUES (?)", ["Buy milk"]);
// r.changes = 1, r.lastInsertId = 1

const open: Note[] = await sql.query<Note>("SELECT id, title, done FROM notes WHERE done = ? ORDER BY id", [false]);

await sql.batch([
  { sql: "UPDATE notes SET done = 1 WHERE id = ?", params: [1] },
  { sql: "DELETE FROM notes WHERE done = 1 AND id < ?", params: [100] },
]); // atomic: all statements or none
```

| Call | Result | Notes |
|---|---|---|
| `sql.exec(statement, params?)` | `Promise<{ changes: number; lastInsertId: number }>` | One statement. |
| `sql.query<T>(statement, params?)` | `Promise<T[]>` | Each row is decoded into `T` as `JSON.parse<T>` decodes a value: a missing or wrong column rejects the promise with a clear message. |
| `sql.batch(steps)` | `Promise<void>` | All steps in one transaction. A failed step rolls back all of them. |
| `sql.migrate(steps)` | `Promise<number>` | Runs the steps after the database's `user_version`, each with its version, in one transaction, and returns the new version. Steps are append-only, like a core ABI. |

**Parameters and values.** A parameter is `number`, `string`, `boolean` (stored as 0 or 1) or `null`. A column value is a number, a string or `null`; `T` can name a `boolean` field, which reads 0 and 1. Binary data (`BLOB`) is not in the first version (SQL-Q3).

**Statements.** The statement is always a string with `?` parameters. The compiler gives warning `PL2026` when the statement argument is not a string literal or a `const` (string building is the usual cause of SQL injection; a dynamic table name is the rare good reason). The check also applies to each statement in an array literal of `batch` and `migrate`. The compiler cannot see the statements of an array value (for example steps made in a loop), so it does not check them.

**Parameters in the compiler.** In an array literal, each parameter is checked alone, so one call can mix the types: `[title, 1, true, null]`. An array value must have one parameter type as its element type (for example `string[]` or `(number | null)[]`), because the language has no union of `number`, `string` and `boolean`. The steps of `batch` are an array literal of `{ sql, params? }` objects, or an array value of an object type with `sql: string` and an optional `params` array.

**Rows in the compiler.** `T` of `query<T>` is an object type (or `Map<string, V>`). The row decoder is the generated decoder of `JSON.parse<T>`, with one change: a `boolean` field reads the numbers 0 and 1 (another number fails), and also `true` and `false`.

**Errors.** A SQL error (a syntax error, a constraint, a busy database) rejects the promise with `Error("sql: <message>")`. A denied call rejects with `Error("denied:<reason>")`, as `files` does. A call never traps.

## 4. Security and limits

1. **One database for each app and instance.** The host derives the path; the app cannot name a file (`docs/STORAGE.md` §3 rule 1).
2. **No way out of the file.** An SQLite authorizer refuses `ATTACH`, `DETACH`, `load_extension`, `VACUUM INTO` and every `PRAGMA` except a safe list (`user_version` read, `foreign_keys`, `table_info`, `index_list`). The host opens the file with `SQLITE_OPEN_NOFOLLOW` and no URI file names.
   - The desktop engine also allows `data_version` (read only). FTS5 reads it in its own statements, and it tells only if the file changed.
   - `VACUUM` attaches a database internally, so the authorizer refuses all `VACUUM` statements, not only `VACUUM INTO`.
   - The authorizer refuses `BEGIN`, `COMMIT`, `ROLLBACK` and `SAVEPOINT`. Use `sql.batch` for a transaction. (A transaction that an app starts and does not end would stop all later writes.)
   - Extension loading is off (the bundled SQLite turns it on by default), the connection is in defensive mode, and it does not trust the schema.
3. **Time limit.** A statement stops after 2 s (an SQLite progress handler), the same limit as an app call. It rejects with `Error("sql: the query took too long")`; the app keeps running.
4. **Size limit.** A quota for each database (`max_page_count`; a default of 256 MiB, and a host policy can change it). A write past the quota rejects with `Error("sql: the database is full")`.
5. **Result size.** A query result over 16 MiB rejects (as `net` has a body limit).
6. **Capability.** `store.sql` (SPEC §11), risk Low (the data stays in the app's own space), with a rationale in `plinth.toml`.

## 5. Core and interfaces

Core 1.12 is shipped and frozen, so this feature is in **core 1.13**.

- WIT: a new interface `sql` with one call `run(kind: u8, statement: string, params: string) -> u32` (a request id). `params` and the result are JSON text. The completion uses the existing 4-field result list (`ok`, an int, `text`, `error`).
- Core 1.13: one new function, `sql_call(thunk, env, kind, statement, params)`, that starts a call (as `files_call` does for `files`). The completion is read with the existing `net_result_*` functions. The decoding of rows into `T` uses the JSON decoder that the compiler already generates for `JSON.parse<T>`, so the core gets no SQL-specific decoder.
- Every host must supply the WIT import `sql.run`, because the core imports it. The web host (`web/plinth-web.js`) answers `denied:unsupported` until step 3.
- The compiler: the `sql` module in `std/store.d.ts`, the generic `query<T>` lowering to the typed JSON decoder, the warning of §3, and the capability.

## 6. Sync and backup

The database is one file in the private space. When the sync engine (`docs/STORAGE.md` §2 item 5) syncs a space, it treats the file as one object: a conflict keeps both versions, and the app (or the user) chooses. A row-level merge is not in the first version (SQL-Q1). Sync comes after 1.0 (owner decision PR-Q5).

## 7. Steps

1. **Done.** The desktop engine: `rusqlite` in the runner's `HostState` on a worker thread, the authorizer, the limits, and unit tests (`crates/plinth-runner-wasmtime/src/sql.rs`). The module comment gives the JSON shapes of the calls.
2. **Done.** WIT `sql.run`, core 1.13 (`sql_call`), the capability `store.sql` (Low), the host import on wasmtime and wasmi, `std/store.d.ts`, the compiler (`crates/plinth-compiler/src/check/sql.rs`, the 0/1 booleans in the row decoder, warning `PL2026`), and `crates/plinth-compiler/tests/sql.rs` on both engines. The web host has a stub that answers `denied:unsupported`.
3. The web engine: SQLite Wasm with OPFS in a Worker of the parent page, the bridge messages, and Node and Edge tests. The same SQL test file runs on both hosts and compares the results.
4. The notes app (VALIDATION V2) uses it for its index and search (SQLite FTS5).
5. Docs: `docs/host-apis.md`, `docs/guides/storage.md`, `docs/LIMITS.md`, `docs/PRODUCTION.md` H4.

## 8. Open questions

| # | Question | Notes |
|---|---|---|
| SQL-Q1 | Row-level sync later (for example the SQLite session extension, or CRDT tables)? | After 1.0, with sync. |
| SQL-Q2 | Can a user open an app's database in a tool to export it ("data belongs to the user")? | A Hub "Export data" action that copies the file is simple and fits owner decision 7. |
| SQL-Q3 | `BLOB` values? | Needs a byte-array type in the language; the `files` API covers most binary data. |
| SQL-Q4 | Full-text search (FTS5) on every host? | The bundled SQLite and the official Wasm build both include FTS5; check the size. |
| SQL-Q5 | One database for each app, or named databases? | One is simpler and enough for V2; named databases can come later with no break. |
| SQL-Q6 | Temporary data (temporary tables, large sorts) uses SQLite temporary files that the quota does not count. | Set `temp_store` to memory and a heap limit (`sqlite3_hard_heap_limit64`) before step 2 opens the API. |
| SQL-Q7 | The authorizer refuses plain `VACUUM` (it attaches a database internally), so an app cannot make its file smaller. | The host can run `VACUUM` itself with the bypass flag, for example from a Hub "Compact" action or when the free pages are more than half the file. |
