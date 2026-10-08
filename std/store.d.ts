// plinth:store: a per-app key/value store (SPEC.md §8.5). Needs the
// `store.kv` capability, declared in plinth.toml.

export declare const kv: {
  /** Returns the value for `key`, or `null` if it is not set. */
  get(key: string): string | null;
  /** Sets `key` to `value`, replacing any existing value. */
  set(key: string, value: string): void;
  /** Removes `key`. Removing a missing key is not an error. */
  remove(key: string): void;
  /** Lists every key currently set, in no particular order. */
  keys(): string[];
  /** The reason the last `kv` call was denied (SPEC.md §8.5): one of
   * `"denied:undeclared"`, `"denied:refused"`, `"denied:unsupported"`, or
   * `null` if the last call was not denied. Cleared to `null` on the next
   * successful `kv` call. A denied call never traps: `get` returns `null`,
   * `set`/`remove` do nothing, and `keys` returns `[]`. */
  lastError(): string | null;
};

// store.sql (core 1.13, docs/SQL.md): one SQLite database for each app.
// Needs the `store.sql` capability (Low risk: granted with no question,
// like `store.kv`). The host keeps the database next to the app's private
// space; no other app can read it.
//
// Every call returns a `Promise`. A SQL error rejects it with an `Error`
// whose message starts with "sql: " (for example "sql: the query took too
// long" or "sql: the database is full"). A denied call rejects with
// "denied:undeclared", "denied:refused" or "denied:unsupported". A failed
// call never traps the app.
//
// Write a statement as a string literal or a `const`, with `?` for each
// value: the compiler gives warning PL2026 for a statement that it cannot
// see (string building is the usual cause of SQL injection). A parameter
// is a `number`, a `string`, a `boolean` (stored as 0 or 1) or `null`.
// `BEGIN`, `COMMIT`, `ATTACH`, `VACUUM` and most `PRAGMA`s are refused: use
// `batch` for a transaction.

export declare const sql: {
  /** Runs one statement. `changes` is the number of rows that it changed;
   * `lastInsertId` is the rowid of the last row that the connection
   * inserted. */
  exec(statement: string, params?: (number | string | boolean | null)[]): Promise<{ changes: number; lastInsertId: number }>;
  /** Runs one statement and gives its rows. Each row is decoded into `T`
   * as `JSON.parse<T>` decodes a value (one field for each column); a
   * `boolean` field also reads 0 and 1. A missing column, or a column of
   * the wrong type, rejects the promise. */
  query<T>(statement: string, params?: (number | string | boolean | null)[]): Promise<T[]>;
  /** Runs all steps in one transaction: all of them or none. */
  batch(steps: { sql: string; params?: (number | string | boolean | null)[] }[]): Promise<void>;
  /** Runs the steps after the database's version (`PRAGMA user_version`) in
   * one transaction, and gives the new version (the number of steps). Steps
   * are append-only: never change or remove a step that ran. */
  migrate(steps: string[]): Promise<number>;
};
