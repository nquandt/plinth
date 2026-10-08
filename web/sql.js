// `store.sql` on the web (core 1.13, docs/SQL.md §2, §4): the same rules,
// limits, JSON shapes and error texts as the desktop engine
// (`crates/plinth-runner-wasmtime/src/sql.rs`), on the official SQLite Wasm
// build (`vendor/sqlite-wasm/`, docs/DEPENDENCIES.md).
//
// Where it runs: an app runs in a Worker of a sandboxed frame with an
// opaque origin, which has no storage. The PARENT page has the origin, so
// the parent page runs SQLite in a dedicated Worker (`sql-worker.js`) with
// the `opfs-sahpool` VFS: one database for each app id, in the OPFS folder
// `.plinth-sql`, which `plinth:files` (IndexedDB) cannot reach. Without
// OPFS (Node, an old browser), each database is in memory and is lost when
// the page closes; the calls still work.
//
// - `SqlEngine`: the databases of one SQLite instance. `call(appId, kind,
//   statement, params)` answers `[ok, int, text, error]` and never throws.
//   No DOM: `test/run-sql.mjs` runs it in Node.
// - `serveSql(scope, start)`: the Worker side (`sql-worker.js`).
// - `SqlWorkerHost`: the page side; `space(appId)` gives the
//   `{ call(kind, statement, params) }` that `AppFrame` and `PlinthApp` use.
// - `localSqlHost(engine)`: the same surface on an engine in this thread
//   (Node tests).

/** The capability of the SQL store (SPEC.md §11). */
export const SQL_CAPABILITY = "store.sql";
/** The time limit of one statement, in ms: the same as an app call. */
export const DEFAULT_STATEMENT_TIMEOUT_MS = 2000;
/** The largest database, in bytes. */
export const DEFAULT_QUOTA = 256 * 1024 * 1024;
/** The largest `query` result, in bytes of JSON text. */
export const DEFAULT_MAX_RESULT = 16 * 1024 * 1024;
/**
 * The most memory of the SQLite instance (the Wasm memory of the SQLite
 * Worker), for all databases together: the desktop `HEAP_LIMIT`. The Wasm
 * build has no `sqlite3_hard_heap_limit64`, so the limit is the maximum
 * size of the Wasm memory; a statement past it fails with `OUT_OF_MEMORY`.
 */
export const HEAP_LIMIT = 512 * 1024 * 1024;

export const TOO_LONG = "sql: the query took too long";
export const FULL = "sql: the database is full";
export const TOO_LARGE = "sql: the result is too large";
export const OUT_OF_MEMORY = "sql: out of memory";
const NOT_AUTHORIZED = "sql: not authorized";
const MULTIPLE = "sql: give one statement only";
const BATCH_SHAPE = 'sql: the batch must be a JSON array of {"sql": string, "params"?: array}';

/** The OPFS folder of the `opfs-sahpool` VFS, and the VFS name. */
export const OPFS_DIRECTORY = ".plinth-sql";
const VFS_NAME = "plinth-sql";

/** How many SQLite virtual machine steps run between two checks of the time limit. */
const PROGRESS_STEPS = 1000;

/**
 * The Emscripten options that start SQLite with the heap limit
 * (`HEAP_LIMIT`). `wasmBinary`: the bytes of `sqlite3.wasm` (Node); in a
 * browser the module finds the file next to itself.
 */
export function sqliteModuleOptions(wasmBinary = null, heapLimit = HEAP_LIMIT) {
  const opts = { wasmMemory: new WebAssembly.Memory({ initial: 256, maximum: Math.max(256, Math.floor(heapLimit / 65536)) }) };
  if (wasmBinary) opts.wasmBinary = wasmBinary;
  return opts;
}

const ok = (int, text) => [true, int, text, null];
const fail = (error) => [false, 0, "", error];

class SqlError extends Error {}
const sqlFail = (text) => {
  throw new SqlError(text);
};

/**
 * A JSON number from the call: `int` (a BigInt) when the JSON text is an
 * integer literal in the i64 range, else `real`. This is the rule of
 * serde_json on the desktop (`as_i64`).
 */
class SqlNumber {
  constructor(value, source) {
    this.real = value;
    this.int = null;
    if (typeof source === "string") {
      if (/^-?\d+$/.test(source)) {
        const big = BigInt(source);
        if (big >= -(2n ** 63n) && big < 2n ** 63n) this.int = big;
      }
    } else if (Number.isInteger(value) && Math.abs(value) < 2 ** 63) {
      // A JSON parser without the source text (an old browser).
      this.int = BigInt(value);
    }
  }
}

/** `JSON.parse`, with each number as a `SqlNumber`. Throws on bad JSON. */
function parseJson(text) {
  return JSON.parse(text, (_key, value, context) => (typeof value === "number" ? new SqlNumber(value, context?.source) : value));
}

const isPlainArray = (v) => Array.isArray(v);
const isPlainObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v) && !(v instanceof SqlNumber);

/** The parameters of `exec` and `query`. Empty text is no parameters. */
function parseParams(text) {
  if (String(text ?? "").trim() === "") return [];
  let v;
  try {
    v = parseJson(text);
  } catch {
    sqlFail("sql: the parameters must be a JSON array");
  }
  if (v === null) return [];
  if (!isPlainArray(v)) sqlFail("sql: the parameters must be a JSON array");
  return v;
}

/** The steps of `batch`: `[[sql, params], …]`. */
function parseBatch(text) {
  let items;
  try {
    items = parseJson(text);
  } catch {
    sqlFail(BATCH_SHAPE);
  }
  if (!isPlainArray(items)) sqlFail(BATCH_SHAPE);
  return items.map((item) => {
    if (!isPlainObject(item) || typeof item.sql !== "string") sqlFail(BATCH_SHAPE);
    const params = Object.hasOwn(item, "params") ? item.params : null;
    if (params === null) return [item.sql, []];
    if (!isPlainArray(params)) sqlFail(BATCH_SHAPE);
    return [item.sql, params];
  });
}

/** The steps of `migrate`: a JSON array of strings. */
function parseMigration(text) {
  let steps;
  try {
    steps = JSON.parse(text);
  } catch {
    sqlFail("sql: the migration steps must be a JSON array of strings");
  }
  if (!Array.isArray(steps) || !steps.every((s) => typeof s === "string")) sqlFail("sql: the migration steps must be a JSON array of strings");
  return steps;
}

/**
 * A REAL value as serde_json writes it: the shortest digits; decimal
 * form when the decimal exponent is from -5 to 15 (an integer value keeps
 * ".0"), else the exponent form "1.5e+16" (JavaScript's own form starts and
 * stops at other exponents). An infinity is null (JSON has no infinity).
 */
export function realJson(f) {
  if (!Number.isFinite(f)) return "null";
  if (Object.is(f, -0)) return "-0.0";
  const exp = f.toExponential(); // the shortest digits: "d.ddde±x"
  const e = Number(exp.slice(exp.indexOf("e") + 1));
  if (e < -5 || e >= 16) return exp;
  const s = String(f); // decimal form for these exponents
  return s.includes(".") ? s : `${s}.0`;
}

/** The UTF-8 length of `s`, without a copy. */
function utf8Length(s) {
  let n = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i);
    if (c < 0x80) n += 1;
    else if (c < 0x800) n += 2;
    else if (c >= 0xd800 && c < 0xdc00 && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
      n += 4;
      i++;
    } else n += 3;
  }
  return n;
}

// Authorizer action codes (sqlite3.h).
const AUTH = { PRAGMA: 19, TRANSACTION: 22, ATTACH: 24, DETACH: 25, FUNCTION: 31, SAVEPOINT: 32, MAX_KNOWN: 33 };
const SQLITE_OK = 0;
const SQLITE_DENY = 1;

/**
 * The authorizer's rule for one action of an app statement (`authorize` in
 * sql.rs). `a` and `b` are the first two text arguments of the action.
 */
export function authorize(code, a, b) {
  switch (code) {
    case AUTH.ATTACH:
    case AUTH.DETACH:
    case AUTH.TRANSACTION:
    case AUTH.SAVEPOINT:
      return SQLITE_DENY;
    case AUTH.FUNCTION:
      return typeof b === "string" && b.toLowerCase() === "load_extension" ? SQLITE_DENY : SQLITE_OK;
    case AUTH.PRAGMA: {
      const name = typeof a === "string" ? a.toLowerCase() : "";
      // No value: the binding gives 0 (a null pointer), not a string.
      if (name === "user_version" || name === "data_version") return typeof b !== "string" ? SQLITE_OK : SQLITE_DENY;
      return ["foreign_keys", "table_info", "index_list"].includes(name) ? SQLITE_OK : SQLITE_DENY;
    }
    default:
      // An action that rusqlite does not know is refused on the desktop.
      return code >= 1 && code <= AUTH.MAX_KNOWN ? SQLITE_OK : SQLITE_DENY;
  }
}

// The primary result codes that rusqlite names (`ErrorCode`); the others
// are `Unknown`, and a prepare error with an `Unknown` code and an offset
// is rusqlite's `SqlInputError`: "<message> in <sql> at offset <n>".
const NAMED_CODES = new Set([2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23, 25, 26]);

/**
 * The databases of one SQLite instance (`sqlite3`, from the init function
 * of the Wasm build). `pool`: the `opfs-sahpool` utility, or null (each
 * database is in memory). `limits`: `{ statementTimeoutMs, quota, maxResult }`.
 */
export class SqlEngine {
  constructor(sqlite3, { pool = null, limits = {} } = {}) {
    this.sqlite3 = sqlite3;
    this.pool = pool;
    this.limits = {
      statementTimeoutMs: limits.statementTimeoutMs ?? DEFAULT_STATEMENT_TIMEOUT_MS,
      quota: limits.quota ?? DEFAULT_QUOTA,
      maxResult: limits.maxResult ?? DEFAULT_MAX_RESULT,
    };
    this.dbs = new Map(); // database name -> SqlDb
  }

  /** True when the databases are in memory (no OPFS). */
  get inMemory() {
    return this.pool === null;
  }

  /** The database name of an app id: one for each app id, in lower case (as `plinth:files` on the web). */
  static databaseName(appId) {
    return `/${encodeURIComponent(String(appId).toLowerCase())}/store.sqlite`;
  }

  async open(appId) {
    const name = SqlEngine.databaseName(appId);
    let db = this.dbs.get(name);
    if (!db) {
      // Each database uses at most two files of the pool: the database and its journal.
      if (this.pool) await this.pool.reserveMinimumCapacity(2 * (this.dbs.size + 1) + 2);
      db = new SqlDb(this.sqlite3, this.pool ? name : ":memory:", this.pool?.vfsName ?? null, this.limits);
      this.dbs.set(name, db);
    }
    return db;
  }

  /** Runs one call (`kind` 0 exec, 1 query, 2 batch, 3 migrate) and gives `[ok, int, text, error]`. */
  async call(appId, kind, statement, params) {
    if (!(kind >= 0 && kind <= 3)) return fail(`sql: unknown call kind ${kind}`);
    let db;
    try {
      db = await this.open(appId);
    } catch (e) {
      return fail(e instanceof SqlError ? e.message : `sql: ${e?.message ?? e}`);
    }
    return db.run(kind, String(statement ?? ""), String(params ?? ""));
  }

  /** The database of `appId`, for tests (`SqlDb`). */
  database(appId) {
    return this.open(appId);
  }
}

/** One open database (`Db` in sql.rs). */
export class SqlDb {
  constructor(sqlite3, filename, vfs, limits) {
    this.sqlite3 = sqlite3;
    this.limits = limits;
    this.deadline = null;
    this.internalOn = false;
    const { capi, wasm } = sqlite3;
    const flags = capi.SQLITE_OPEN_READWRITE | capi.SQLITE_OPEN_CREATE | capi.SQLITE_OPEN_NOFOLLOW | capi.SQLITE_OPEN_EXRESCODE;
    const stack = wasm.pstack.pointer;
    try {
      const pp = wasm.pstack.allocPtr();
      const rc = capi.sqlite3_open_v2(filename, pp, flags, vfs);
      this.p = wasm.peekPtr(pp);
      if (rc !== 0) {
        const msg = this.p ? capi.sqlite3_errmsg(this.p) : capi.sqlite3_errstr(rc);
        if (this.p) capi.sqlite3_close_v2(this.p);
        sqlFail(`sql: ${msg}`);
      }
    } finally {
      wasm.pstack.restore(stack);
    }
    this.configure();
  }

  /** The settings before the app's first statement (`Db::configure`). The authorizer goes on last. */
  configure() {
    const { capi } = this.sqlite3;
    const p = this.p;
    // The Wasm build has no extension loading (SQLITE_OMIT_LOAD_EXTENSION).
    this.check(capi.sqlite3_db_config(p, capi.SQLITE_DBCONFIG_DEFENSIVE, 1, 0));
    this.check(capi.sqlite3_db_config(p, capi.SQLITE_DBCONFIG_TRUSTED_SCHEMA, 0, 0));
    capi.sqlite3_limit(p, capi.SQLITE_LIMIT_ATTACHED, 0);
    // Temporary data stays in memory (docs/SQL.md SQL-Q6), under `HEAP_LIMIT`.
    this.internal("PRAGMA temp_store = MEMORY");
    const pageSize = Number(this.internalValue("PRAGMA page_size"));
    const maxPages = Math.max(1, Math.floor(this.limits.quota / Math.max(pageSize, 512)));
    this.internal(`PRAGMA max_page_count = ${maxPages}`);
    this.check(
      capi.sqlite3_progress_handler(
        p,
        PROGRESS_STEPS,
        () => (this.deadline !== null && performance.now() >= this.deadline ? 1 : 0),
        0,
      ),
    );
    this.check(capi.sqlite3_set_authorizer(p, (_pv, code, a, b) => (this.internalOn ? SQLITE_OK : authorize(code, a, b)), 0));
  }

  check(rc) {
    // Some bindings give `undefined` (a C function with no result).
    if (rc) sqlFail(this.errorText(rc));
  }

  /** The error text of a SQLite result code (`sql_error` in sql.rs). */
  errorText(rc, sql = null) {
    const { capi } = this.sqlite3;
    const primary = rc & 0xff;
    if (primary === capi.SQLITE_INTERRUPT) return TOO_LONG;
    if (primary === capi.SQLITE_FULL) return FULL;
    if (primary === capi.SQLITE_NOMEM) return OUT_OF_MEMORY;
    if (primary === capi.SQLITE_AUTH) return NOT_AUTHORIZED;
    const msg = capi.sqlite3_errcode(this.p) !== rc ? capi.sqlite3_errstr(rc) : capi.sqlite3_errmsg(this.p);
    if (sql !== null && !NAMED_CODES.has(primary)) {
      const offset = capi.sqlite3_error_offset(this.p);
      if (offset >= 0) return `sql: ${msg} in ${sql} at offset ${offset}`;
    }
    return `sql: ${msg}`;
  }

  /** Runs a statement of the host itself, with the authorizer off (`Db::internal`). */
  internal(sql) {
    this.internalOn = true;
    try {
      const stmt = this.prepare(sql);
      try {
        this.stepAll(stmt);
      } finally {
        this.sqlite3.capi.sqlite3_finalize(stmt);
      }
    } finally {
      this.internalOn = false;
    }
  }

  /** The first column of the first row of a host statement. */
  internalValue(sql) {
    const { capi } = this.sqlite3;
    this.internalOn = true;
    try {
      const stmt = this.prepare(sql);
      try {
        const rc = capi.sqlite3_step(stmt);
        if (rc === capi.SQLITE_ROW) return capi.sqlite3_column_int64(stmt, 0);
        if (rc !== capi.SQLITE_DONE) sqlFail(this.errorText(rc));
        return null;
      } finally {
        capi.sqlite3_finalize(stmt);
      }
    } finally {
      this.internalOn = false;
    }
  }

  /**
   * Prepares one statement (`Connection::prepare` of rusqlite): a second
   * statement after it is an error, an empty statement gives 0.
   */
  prepare(sql) {
    const { capi, wasm } = this.sqlite3;
    const [pSql, n] = wasm.allocCString(sql, true);
    const stack = wasm.pstack.pointer;
    try {
      const ppStmt = wasm.pstack.allocPtr();
      const pzTail = wasm.pstack.allocPtr();
      const rc = capi.sqlite3_prepare_v3(this.p, pSql, n, 0, ppStmt, pzTail);
      if (rc !== 0) sqlFail(this.errorText(rc, sql));
      const stmt = wasm.peekPtr(ppStmt);
      const pTail = wasm.peekPtr(pzTail);
      const tail = pTail ? Number(pTail) - Number(pSql) : 0;
      if (tail > 0 && tail < n) {
        const rest = new TextDecoder().decode(wasm.heap8u().slice(Number(pTail), Number(pSql) + n));
        let next;
        try {
          next = this.prepare(rest);
        } catch (e) {
          if (stmt) capi.sqlite3_finalize(stmt);
          throw e;
        }
        if (next) {
          capi.sqlite3_finalize(next);
          if (stmt) capi.sqlite3_finalize(stmt);
          sqlFail(MULTIPLE);
        }
      }
      return stmt;
    } finally {
      wasm.pstack.restore(stack);
      wasm.dealloc(pSql);
    }
  }

  /** Binds `params` to the `?` positions (`bind` in sql.rs). */
  bind(stmt, params) {
    const { capi } = this.sqlite3;
    const expected = stmt ? capi.sqlite3_bind_parameter_count(stmt) : 0;
    if (expected !== params.length) sqlFail(`sql: the statement has ${expected} parameters, but the call gives ${params.length}`);
    params.forEach((p, i) => {
      const at = i + 1;
      let rc;
      if (p === null) rc = capi.sqlite3_bind_null(stmt, at);
      else if (typeof p === "boolean") rc = capi.sqlite3_bind_int64(stmt, at, p ? 1n : 0n);
      else if (p instanceof SqlNumber) rc = p.int !== null ? capi.sqlite3_bind_int64(stmt, at, p.int) : capi.sqlite3_bind_double(stmt, at, p.real);
      else if (typeof p === "number") rc = Number.isInteger(p) ? capi.sqlite3_bind_int64(stmt, at, BigInt(p)) : capi.sqlite3_bind_double(stmt, at, p);
      else if (typeof p === "string") {
        // The C string is given as a pointer: the string form of the
        // binding has a bug in this build (it reads an undefined name).
        // SQLite frees it (`SQLITE_WASM_DEALLOC`), also when the call fails.
        const [ptr, n] = this.sqlite3.wasm.allocCString(p, true);
        rc = capi.sqlite3_bind_text(stmt, at, ptr, n, capi.SQLITE_WASM_DEALLOC);
      }
      else sqlFail(`sql: parameter ${at} must be a number, a string, a boolean or null`);
      if (rc) sqlFail(this.errorText(rc));
    });
  }

  /** Steps `stmt` to the end, with the time limit on when `timed`. Calls `onRow` for each row. */
  stepAll(stmt, onRow = null, timed = false) {
    if (!stmt) return;
    const { capi } = this.sqlite3;
    if (timed) this.deadline = performance.now() + this.limits.statementTimeoutMs;
    try {
      for (;;) {
        const rc = capi.sqlite3_step(stmt);
        if (rc === capi.SQLITE_ROW) {
          if (onRow && onRow() === false) return;
          continue;
        }
        if (rc === capi.SQLITE_DONE) return;
        sqlFail(this.errorText(rc));
      }
    } finally {
      if (timed) this.deadline = null;
    }
  }

  /** `exec(statement, params)`: `{ changes, lastInsertId }` as JSON numbers in text. */
  execValues(sql, params) {
    const { capi } = this.sqlite3;
    const stmt = this.prepare(sql);
    try {
      this.bind(stmt, params);
      const readonly = stmt ? capi.sqlite3_stmt_readonly(stmt) !== 0 : true;
      this.stepAll(stmt, null, true);
      const changes = readonly ? 0n : BigInt(capi.sqlite3_changes64(this.p));
      return { changes, lastInsertId: BigInt(capi.sqlite3_last_insert_rowid(this.p)) };
    } finally {
      if (stmt) capi.sqlite3_finalize(stmt);
    }
  }

  exec(sql, paramsJson) {
    return this.execValues(sql, parseParams(paramsJson));
  }

  /** `query(statement, params)`: a JSON array of row objects. */
  query(sql, paramsJson) {
    const { capi } = this.sqlite3;
    const params = parseParams(paramsJson);
    const stmt = this.prepare(sql);
    try {
      this.bind(stmt, params);
      const count = stmt ? capi.sqlite3_column_count(stmt) : 0;
      const names = [];
      for (let i = 0; i < count; i++) names.push(capi.sqlite3_column_name(stmt, i));
      // The keys of a row in column order; a repeated name keeps its first
      // place and its last value (as on the desktop).
      const order = [...new Set(names)];
      const max = this.limits.maxResult;
      const rows = [];
      let size = 1; // "["
      let failure = null;
      this.stepAll(
        stmt,
        () => {
          const values = new Map();
          for (let i = 0; i < count; i++) {
            let v;
            switch (capi.sqlite3_column_type(stmt, i)) {
              case capi.SQLITE_INTEGER:
                v = String(capi.sqlite3_column_int64(stmt, i));
                break;
              case capi.SQLITE_FLOAT:
                v = realJson(capi.sqlite3_column_double(stmt, i));
                break;
              case capi.SQLITE_TEXT:
                v = JSON.stringify(capi.sqlite3_column_text(stmt, i) ?? "");
                break;
              case capi.SQLITE_BLOB:
                failure = `sql: the column "${names[i]}" is a BLOB, which store.sql cannot read`;
                return false;
              default:
                v = "null";
            }
            values.set(names[i], v);
          }
          const row = `{${order.map((k) => `${JSON.stringify(k)}:${values.get(k)}`).join(",")}}`;
          size += utf8Length(row) + (rows.length ? 1 : 0);
          rows.push(row);
          if (size + 1 > max) {
            failure = TOO_LARGE;
            return false;
          }
          return true;
        },
        true,
      );
      if (failure) sqlFail(failure);
      return `[${rows.join(",")}]`;
    } finally {
      if (stmt) capi.sqlite3_finalize(stmt);
    }
  }

  /** `batch(steps)`: all steps in one transaction; a failed step rolls back all of them. */
  batch(stepsJson) {
    const steps = parseBatch(stepsJson);
    this.transaction(() => {
      for (const [sql, params] of steps) this.execValues(sql, params);
    });
  }

  /** The database's `user_version` (read with the authorizer on, as on the desktop). */
  version() {
    const { capi } = this.sqlite3;
    const stmt = this.prepare("PRAGMA user_version");
    try {
      const rc = capi.sqlite3_step(stmt);
      if (rc !== capi.SQLITE_ROW) sqlFail(this.errorText(rc));
      return Number(capi.sqlite3_column_int64(stmt, 0));
    } finally {
      capi.sqlite3_finalize(stmt);
    }
  }

  /** `migrate(steps)`: runs the steps after `user_version` in one transaction; gives the new version. */
  migrate(stepsJson) {
    const steps = parseMigration(stepsJson);
    const target = steps.length;
    const current = this.version();
    if (current > target) {
      sqlFail(`sql: the database is at version ${current}, but there are only ${target} migration steps (steps are append-only)`);
    }
    if (current === target) return current;
    this.transaction(() => {
      for (const sql of steps.slice(current)) this.execValues(sql, []);
      this.internal(`PRAGMA user_version = ${target}`);
    });
    return target;
  }

  transaction(f) {
    this.internal("BEGIN IMMEDIATE");
    try {
      f();
    } catch (e) {
      // SQLite can roll back by itself (after an interrupt or a full
      // database); then this ROLLBACK fails, which is correct.
      try {
        this.internal("ROLLBACK");
      } catch {}
      throw e;
    }
    try {
      this.internal("COMMIT");
    } catch (e) {
      try {
        this.internal("ROLLBACK");
      } catch {}
      throw e;
    }
  }

  /** Runs one call and gives its completion `[ok, int, text, error]` (`Db::run`). */
  run(kind, statement, params) {
    try {
      switch (kind) {
        case 0: {
          const r = this.execValues(statement, parseParams(params));
          return ok(Number(BigInt.asIntN(32, r.changes)), `{"changes":${r.changes},"lastInsertId":${r.lastInsertId}}`);
        }
        case 1:
          return ok(0, this.query(statement, params));
        case 2:
          this.batch(params);
          return ok(0, "null");
        case 3: {
          const v = this.migrate(params);
          return ok(v | 0, String(v));
        }
        default:
          return fail(`sql: unknown call kind ${kind}`);
      }
    } catch (e) {
      return fail(e instanceof SqlError ? e.message : `sql: ${e?.message ?? e}`);
    }
  }

  close() {
    this.sqlite3.capi.sqlite3_close_v2(this.p);
  }
}

/**
 * Starts SQLite in this thread and makes the engine: with the
 * `opfs-sahpool` VFS when the thread has OPFS (a dedicated Worker of a page
 * with an origin), else in memory. `init` is the default export of the
 * Wasm build; `options` are its Emscripten options (`sqliteModuleOptions`).
 */
export async function startSqlEngine(init, options = sqliteModuleOptions(), limits = {}) {
  const sqlite3 = await init(options);
  const hasOpfs = typeof globalThis.FileSystemSyncAccessHandle === "function" && typeof globalThis.navigator?.storage?.getDirectory === "function";
  let pool = null;
  if (hasOpfs) {
    // A failure here (for example another tab of this origin holds the
    // files) rejects: a database in memory would lose the app's data.
    pool = await sqlite3.installOpfsSAHPoolVfs({ name: VFS_NAME, directory: OPFS_DIRECTORY, initialCapacity: 4 });
  }
  return new SqlEngine(sqlite3, { pool, limits });
}

/**
 * The Worker side: answers each `{ id, appId, kind, statement, params }`
 * with `{ id, result }`, in the order of the messages. `start()` gives the
 * engine (a promise); when it fails, every call answers an error.
 */
export function serveSql(scope, start) {
  let engine = null;
  let queue = Promise.resolve();
  const ready = Promise.resolve()
    .then(start)
    .then(
      (e) => (engine = e),
      (err) => {
        engine = { failed: `sql: the database engine did not start: ${err?.message ?? err}` };
      },
    );
  scope.onmessage = (event) => {
    const m = event.data;
    if (!m || typeof m.id !== "number") return;
    queue = queue
      .then(() => ready)
      .then(() => (engine.failed ? fail(engine.failed) : engine.call(m.appId, m.kind, m.statement, m.params)))
      .catch((e) => fail(`sql: ${e?.message ?? e}`))
      .then((result) => scope.postMessage({ id: m.id, result, memory: engine?.inMemory ?? null }));
  };
}

/**
 * The page side: one SQLite Worker for the page (`workerUrl`, a module
 * Worker of `sql-worker.js`), started at the first call. `space(appId)`
 * gives the backend of one app. `makeWorker(url)` is for tests.
 */
export class SqlWorkerHost {
  constructor(workerUrl, makeWorker = (url) => new Worker(url, { type: "module", name: "plinth-sql" })) {
    this.workerUrl = workerUrl;
    this.makeWorker = makeWorker;
    this.worker = null;
    this.nextId = 1;
    this.pending = new Map();
    /** Null until the first answer; then true when the databases are in memory. */
    this.inMemory = null;
  }

  start() {
    if (this.worker) return this.worker;
    const worker = this.makeWorker(this.workerUrl);
    worker.onmessage = (event) => {
      const { id, result, memory } = event.data ?? {};
      if (memory !== null && memory !== undefined && this.inMemory === null) {
        this.inMemory = memory;
        if (memory) console.warn("[plinth] store.sql: this browser has no OPFS; the databases are in memory and are lost when the page closes.");
      }
      const resolve = this.pending.get(id);
      if (!resolve) return;
      this.pending.delete(id);
      resolve(result);
    };
    worker.onerror = (event) => {
      event?.preventDefault?.();
      const error = `sql: the database engine did not start: ${event?.message ?? "error"}`;
      for (const resolve of this.pending.values()) resolve(fail(error));
      this.pending.clear();
      this.failed = error;
    };
    this.worker = worker;
    return worker;
  }

  call(appId, kind, statement, params) {
    if (this.failed) return Promise.resolve(fail(this.failed));
    const worker = this.start();
    const id = this.nextId++;
    return new Promise((resolve) => {
      this.pending.set(id, resolve);
      worker.postMessage({ id, appId, kind, statement, params });
    });
  }

  space(appId) {
    return { call: (kind, statement, params) => this.call(appId, kind, statement, params) };
  }
}

/** The surface of `SqlWorkerHost` on an engine in this thread (Node tests); calls run in call order. */
export function localSqlHost(engine) {
  let queue = Promise.resolve();
  const call = (appId, kind, statement, params) => {
    const run = queue.then(() => engine.call(appId, kind, statement, params));
    queue = run.catch(() => {});
    return run;
  };
  return { call, space: (appId) => ({ call: (kind, statement, params) => call(appId, kind, statement, params) }) };
}
