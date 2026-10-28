# SQL store design (draft)

This document is the design of `store.sql`: a SQL database for each app. It is checklist item H4 in `docs/PRODUCTION.md`. The owner decided on 2026-10-07 that Plinth gives apps a SQL store (`docs/STORAGE.md` ST-Q1).

Status: 2026-10-08. All five steps of §7 are done (PRODUCTION H4 is Done): apps call `store.sql` on the desktop host (wasmtime and wasmi, core 1.13) and on the web host (the official SQLite Wasm build in a Worker of the page, with OPFS), and `examples/notes` keeps its notes and its search index in it. §9 gives the differences of the web engine. The folder form of a web export (`plinth build --target web`) has it too (§9 W11). A single-file export does not: there each call rejects with `Error("denied:unsupported")`.

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
| Web | The official SQLite Wasm build (`web/vendor/sqlite-wasm/`, `docs/DEPENDENCIES.md`) in a dedicated Worker of the **parent page** (the host shell), with the OPFS VFS `opfs-sahpool` (`web/sql.js`, `web/sql-worker.js`) | OPFS of the page origin, folder `.plinth-sql`, one database for each app id in lower case (`/<app id>/store.sqlite` in the pool). `plinth:files` on the web keeps its files in IndexedDB, so it cannot reach the database. Without OPFS (Node, an old browser), each database is in memory and goes away when the page closes. |

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
4. **Size limit.** A quota for each database (`max_page_count`). The desktop host uses the data quota of the app (100 MiB by default; `docs/STORAGE.md` §7.3), the web host 256 MiB. A write past the quota rejects with `Error("sql: the database is full")`.
5. **Result size.** A query result over 16 MiB rejects (as `net` has a body limit).
6. **Capability.** `store.sql` (SPEC §11), risk Low (the data stays in the app's own space), with a rationale in `plinth.toml`.

## 5. Core and interfaces

Core 1.12 is shipped and frozen, so this feature is in **core 1.13**.

- WIT: a new interface `sql` with one call `run(kind: u8, statement: string, params: string) -> u32` (a request id). `params` and the result are JSON text. The completion uses the existing 4-field result list (`ok`, an int, `text`, `error`).
- Core 1.13: one new function, `sql_call(thunk, env, kind, statement, params)`, that starts a call (as `files_call` does for `files`). The completion is read with the existing `net_result_*` functions. The decoding of rows into `T` uses the JSON decoder that the compiler already generates for `JSON.parse<T>`, so the core gets no SQL-specific decoder.
- Every host must supply the WIT import `sql.run`, because the core imports it. The web host (`web/plinth-web.js`) sends the call to the page as the bridge message `sql-run` (`web/README.md`).
- The compiler: the `sql` module in `std/store.d.ts`, the generic `query<T>` lowering to the typed JSON decoder, the warning of §3, and the capability.

## 6. Sync and backup

The database is one file in the private space. When the sync engine (`docs/STORAGE.md` §2 item 5) syncs a space, it treats the file as one object: a conflict keeps both versions, and the app (or the user) chooses. A row-level merge is not in the first version (SQL-Q1). Sync comes after 1.0 (owner decision PR-Q5).

## 7. Steps

1. **Done.** The desktop engine: `rusqlite` in the runner's `HostState` on a worker thread, the authorizer, the limits, and unit tests (`crates/plinth-runner-wasmtime/src/sql.rs`). The module comment gives the JSON shapes of the calls.
2. **Done.** WIT `sql.run`, core 1.13 (`sql_call`), the capability `store.sql` (Low), the host import on wasmtime and wasmi, `std/store.d.ts`, the compiler (`crates/plinth-compiler/src/check/sql.rs`, the 0/1 booleans in the row decoder, warning `PL2026`), and `crates/plinth-compiler/tests/sql.rs` on both engines. The web host has a stub that answers `denied:unsupported`.
3. **Done.** The web engine (`web/sql.js`, `web/sql-worker.js`): SQLite Wasm 3.53.4 with `opfs-sahpool` in a Worker of the parent page, the same authorizer, limits and error texts as the desktop, and the bridge message `sql-run`. The shared case file `crates/plinth-compiler/tests/sql_cases.txt` runs on both hosts (`tests/sql.rs` and `web/test/run-sql.mjs`), and the results must be the same text. `web/test/run-sql.mjs` also runs the cases of the desktop unit tests on the web engine. `web/test/run-a11y.mjs` (by hand, Edge) runs a fixture app in the web App Hub: the data stays in OPFS after a reload of the page.
4. **Done.** The notes app (`examples/notes`, VALIDATION V2) keeps its notes in a table and searches them with SQLite FTS5:
   - `app/storage.ts`: the table `notes`, the FTS5 table `notes_fts` (external content, kept in step by three triggers), and a change counter `changed` for the sort by the last change. The search box gives a prefix search on each word: each word becomes an FTS5 string with `*` (a `"` in it is doubled), so a quote or an FTS5 operator (`*`, `-`, `AND`, `NEAR(`, `^`) is text, never syntax.
   - On the first run after the change, the app moves the notes of an older build from `store.kv` (one JSON value) into the empty table in one `batch`, and then removes the kv key. For this, the app keeps `store.kv` in its capabilities.
   - Tests: `crates/plinth-compiler/tests/notes.rs` (desktop) and `web/test/run-notes.mjs` (web): add, edit, delete, search, the next run, the kv move, search text with quotes and operators, and 1,000 notes. With 1,000 notes, a search takes about 2 ms to 10 ms on the desktop (debug build) and 1 ms to 5 ms on the web (Node), from the change of the search text to the new list.
   - Two defects were found and fixed: `JSON.stringify` of a large array or map ran out of memory (the collector does not run during a call, and each part made a longer copy of the text; now the parts are joined one time), and on the web host a `kv.get` after a memory growth wrote to a detached buffer.
5. Docs: `docs/host-apis.md`, `docs/guides/storage.md`, `docs/LIMITS.md`, `docs/PRODUCTION.md` H4. **Done.**

## 8. Open questions

| # | Question | Notes |
|---|---|---|
| SQL-Q1 | Row-level sync later (for example the SQLite session extension, or CRDT tables)? | After 1.0, with sync. |
| SQL-Q2 | Can a user open an app's database in a tool to export it ("data belongs to the user")? | A Hub "Export data" action that copies the file is simple and fits owner decision 7. |
| SQL-Q3 | `BLOB` values? | Needs a byte-array type in the language; the `files` API covers most binary data. |
| SQL-Q4 | Full-text search (FTS5) on every host? | The bundled SQLite and the official Wasm build both include FTS5; check the size. |
| SQL-Q5 | One database for each app, or named databases? | One is simpler and enough for V2; named databases can come later with no break. |
| SQL-Q6 | Temporary data (temporary tables, large sorts) used SQLite temporary files that the quota does not count. | Done on the desktop: `temp_store = MEMORY`, which an app cannot change, and a heap limit of 512 MiB for the whole host process (`sql::HEAP_LIMIT`; SQLite has no limit for each database). A statement past it fails with `sql: out of memory`. Done on the web: `temp_store = MEMORY`, and the Wasm memory of the SQLite Worker has a maximum of 512 MiB (`HEAP_LIMIT` in `web/sql.js`; the Wasm build has no `sqlite3_hard_heap_limit64`). |
| SQL-Q7 | The authorizer refuses plain `VACUUM` (it attaches a database internally), so an app cannot make its file smaller. | The host can run `VACUUM` itself with the bypass flag, for example from a Hub "Compact" action or when the free pages are more than half the file. |

## 9. The web engine: differences from the desktop

The web engine has the same rules, limits, JSON shapes and error texts as the desktop engine. `web/test/run-sql.mjs` checks this. These things are different:

| # | Web | Desktop | Why |
|---|---|---|---|
| W1 | One SQLite Worker for the page runs the calls of all apps, one at a time. A statement of one app (at most 2 s) delays the calls of the other apps on the page. | One worker thread for each app. | One Wasm instance and one OPFS pool for the page. Usually one app runs at a time. |
| W2 | The memory limit is the maximum size of the Wasm memory of the SQLite Worker: 512 MiB for all databases, with SQLite's own data. | `sqlite3_hard_heap_limit64` (512 MiB for the host process). | The Wasm build does not export the heap limit function. The error text is the same: `sql: out of memory`. |
| W3 | The Wasm build has no extension loading at all (`SQLITE_OMIT_LOAD_EXTENSION`). `SELECT load_extension('x')` fails with `sql: no such function: load_extension`. | The authorizer refuses it: `sql: not authorized`. | Both refuse it; only the text is different. |
| W4 | Without OPFS (Node, an old browser), each database is in memory and goes away when the page closes. The page writes a warning to the console one time. The calls work. | Always a file. | A page without OPFS has no synchronous file access for SQLite. |
| W5 | `opfs-sahpool` keeps the files open with exclusive access. A second tab of the same origin cannot open the pool while the first tab has it; its calls fail with `sql: the database engine did not start: <reason>`. (Not tested yet.) | Each host process opens the file; SQLite locks it. | A limit of `opfs-sahpool`. The `opfs` VFS, which allows more tabs, needs a cross-origin isolated page (COOP and COEP headers). |
| W6 | The database name is the app id in lower case, with no owner part (the same rule as `plinth:files` on the web). | `<owner>/<app id>+sql/store.sqlite`. | The web host has one owner for each origin. |
| W7 | A statement or a parameter text longer than 16 Mi characters fails at once with `sql: the call is too large` (`MAX_SQL_TEXT` in `web/plinth-web.js`). | No limit (only memory). | The text crosses two `postMessage` bridges (Worker to frame to page). |
| W8 | The page size of a new database is 8192 bytes. | 4096 bytes. | The default of each build. The quota is in bytes (`max_page_count` comes from the page size), so the limit is the same. |
| W9 | SQLite 3.53.4. | SQLite 3.53.2 (`libsqlite3-sys` 0.38.2). | Each build has its own SQLite. The shared cases give the same text on both. |
| W10 | An integer parameter binds as an integer when its JSON text is an integer in the i64 range, as `serde_json` does on the desktop. A browser without the source text in `JSON.parse` revivers (Chrome before 114, Firefox before 135) binds each number with no fraction as an integer. | `serde_json`. | The core writes the parameters as JSON text. |
| W11 | The folder form of a web export (`plinth build --target web`) has `store.sql` when the app declares it: the export writes `sql-worker.js`, `sql.js` and `vendor/sqlite-wasm/` (about 1.5 MB) next to `plinth.js`, and the element gets `sql="sql-worker.js"`. The database is in OPFS of the page origin and stays after a reload. An app that does not declare `store.sql` gets none of these files. A single-file export (`--single-file`) does not have `store.sql`: an allowed call answers `denied:unsupported`, and `plinth build` writes a warning. | Not applicable. | A single file cannot start a module Worker from a file next to it or load the Wasm build from a file. To put the Worker (a `blob:` URL) and the SQLite files (about 1.8 MB more with base64) inline is possible, but it is not done. |

The row text: each row is a JSON object with its keys in column order. A repeated column name keeps its first place and its last value. A REAL value is written as `serde_json` writes it (for example `2.0`, `1e+16`, `1e-6`), and an INTEGER value keeps all its digits. The desktop engine writes the same text. (It does not use `serde_json::Map`, because the key order of that map changes with the `preserve_order` feature, which another crate of a build can turn on.)
