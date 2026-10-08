// Node test: `store.sql` on the web host (core 1.13, docs/SQL.md §2-§4,
// step 3 of §7). The engine (`sql.js` on the official SQLite Wasm build,
// in memory: Node has no OPFS) with the cases of the desktop unit tests
// (`crates/plinth-runner-wasmtime/src/sql.rs`): the same results, limits
// and error texts. The Worker protocol over a `MessageChannel`, the strict
// check of the `sql-run` bridge message, and then the SAME app cases as the
// desktop test `crates/plinth-compiler/tests/sql.rs` (the shared file
// `crates/plinth-compiler/tests/sql_cases.txt`): each case is built by the
// CLI and runs on the real core, and its result must be the desktop result.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import init from "../vendor/sqlite-wasm/sqlite3.mjs";
import {
  SqlEngine,
  startSqlEngine,
  sqliteModuleOptions,
  serveSql,
  SqlWorkerHost,
  localSqlHost,
  TOO_LONG,
  FULL,
  TOO_LARGE,
  OUT_OF_MEMORY,
  HEAP_LIMIT,
  realJson,
} from "../sql.js";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { runWorker, WorkerApp } from "../app-worker.js";
import { checkFrameMessage, CHANNEL } from "../hub-storage.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const wasmBinary = readFileSync(path.join(root, "web/vendor/sqlite-wasm/sqlite3.wasm"));

// The Wasm build warns that it cannot install its "opfs" VFS in Node.
globalThis.sqlite3ApiConfig = { warn: () => {}, log: () => {}, debug: () => {}, error: console.error };

async function engine(limits = {}, heapLimit = HEAP_LIMIT) {
  globalThis.sqlite3ApiConfig = { warn: () => {}, log: () => {}, debug: () => {}, error: () => {} };
  return startSqlEngine(init, sqliteModuleOptions(wasmBinary, heapLimit), limits);
}

const okText = (r) => {
  assert.equal(r[0], true, `expected success, got ${JSON.stringify(r)}`);
  return r[2];
};
const errText = (r) => {
  assert.equal(r[0], false, `expected an error, got ${JSON.stringify(r)}`);
  return r[3];
};

// The same statement and text as `row_keys_are_in_column_order` in sql.rs.
const REALS = "2.0 AS c, 1e308 * 10 AS d, 9007199254740993 AS e, 1e21 AS f, 1.5e-7 AS g, 0.1 AS h, -0.0 AS i, 123456789012345680000.0 AS j";
const REALS_JSON = '"c":2.0,"d":null,"e":9007199254740993,"f":1e+21,"g":1.5e-7,"h":0.1,"i":-0.0,"j":1.2345678901234568e+20';

/** REAL values: the text of serde_json 1.0.151 on the desktop for each value. */
function reals() {
  const cases = [
    [1e15, "1000000000000000.0"],
    [1e16, "1e+16"],
    [123456789012345.6, "123456789012345.6"],
    [1234567890123456.7, "1234567890123456.8"],
    [12345678901234567.0, "1.2345678901234568e+16"],
    [0.001, "0.001"],
    [0.00001, "0.00001"],
    [1e-6, "1e-6"],
    [1.5e-6, "1.5e-6"],
    [2.5, "2.5"],
    [100.0, "100.0"],
    [-100.0, "-100.0"],
    [1e22, "1e+22"],
    [5e-324, "5e-324"],
    [1.7976931348623157e308, "1.7976931348623157e+308"],
    [Infinity, "null"],
  ];
  for (const [f, text] of cases) assert.equal(realJson(f), text, String(f));
}

/** The engine with the cases of the unit tests in sql.rs. */
async function engineCases() {
  const e = await engine();
  assert.equal(e.inMemory, true, "Node has no OPFS");
  const call = (kind, statement, params = "") => e.call("com.example.sql", kind, statement, params);
  const exec = (s, p) => call(0, s, p);
  const query = (s, p) => call(1, s, p);

  // exec_and_query_round_trip
  okText(await exec("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, score REAL)"));
  let r = await exec("INSERT INTO notes (title, score) VALUES (?, ?)", '["Buy milk", 1.5]');
  assert.deepEqual(r, [true, 1, '{"changes":1,"lastInsertId":1}', null]);
  r = await exec("INSERT INTO notes (title, done) VALUES (?, ?)", '["Call Bob", true]');
  assert.equal(JSON.parse(okText(r)).lastInsertId, 2);
  assert.equal(
    okText(await query("SELECT id, title, done, score FROM notes ORDER BY id", "[]")),
    '[{"id":1,"title":"Buy milk","done":0,"score":1.5},{"id":2,"title":"Call Bob","done":1,"score":null}]',
  );
  assert.equal(okText(await query("SELECT title FROM notes WHERE done = ?", "[false]")), '[{"title":"Buy milk"}]');
  assert.equal(okText(await query("SELECT * FROM notes WHERE id > 99")), "[]");
  assert.equal(JSON.parse(okText(await exec("UPDATE notes SET done = 1"))).changes, 2);
  assert.equal(JSON.parse(okText(await exec("SELECT 1"))).changes, 0);

  // params_of_each_type
  assert.equal(
    okText(await query("SELECT ? AS i, ? AS f, ? AS s, ? AS t, ? AS b, ? AS n", '[42, -2.25, "héllo", true, false, null]')),
    '[{"i":42,"f":-2.25,"s":"héllo","t":1,"b":0,"n":null}]',
  );
  for (const bad of ["[[1]]", '[{"a":1}]']) assert.match(errText(await query("SELECT ?", bad)), /must be a number, a string, a boolean or null/);
  assert.match(errText(await query("SELECT ?", '{"a":1}')), /JSON array/);
  assert.match(errText(await query("SELECT ?", "[1, 2]")), /has 1 parameters/);
  assert.match(errText(await query("SELECT ?, ?", "[1]")), /has 2 parameters/);
  // Integer and REAL values as serde_json writes them; i64 stays exact.
  assert.equal(
    okText(await query(`SELECT ? AS a, ? AS b, ${REALS}`, "[3, 3.5]")),
    `[{"a":3,"b":3.5,${REALS_JSON}}]`,
  );
  assert.equal(okText(await query("SELECT ? AS big", "[9007199254740993]")), '[{"big":9007199254740993}]');

  // errors_are_sql_errors
  assert.match(errText(await exec("CREATE TABLEX t (a)")), /^sql: /);
  assert.equal(errText(await exec("SELEC 1")), 'sql: near "SELEC": syntax error in SELEC 1 at offset 0');
  okText(await exec("CREATE TABLE t (a INTEGER UNIQUE)"));
  okText(await exec("INSERT INTO t VALUES (1)"));
  assert.equal(errText(await exec("INSERT INTO t VALUES (1)")), "sql: UNIQUE constraint failed: t.a");
  assert.equal(errText(await exec("INSERT INTO t VALUES (2); INSERT INTO t VALUES (3)")), "sql: give one statement only");
  assert.equal(errText(await call(7, "SELECT 1")), "sql: unknown call kind 7");

  // blob_columns_are_an_error
  assert.match(errText(await query("SELECT x'00ff' AS data")), /"data" is a BLOB/);

  // authorizer_refuses_the_ways_out
  const refused = [
    "ATTACH 'other.sqlite' AS o",
    "DETACH main",
    "PRAGMA writable_schema = ON",
    "PRAGMA writable_schema",
    "PRAGMA user_version = 5",
    "PRAGMA max_page_count = 100000000",
    "PRAGMA journal_mode = OFF",
    "PRAGMA temp_store = FILE",
    "VACUUM INTO 'other.sqlite'",
    "VACUUM",
    "SELECT load_extension('x')",
    "BEGIN",
    "COMMIT",
    "SAVEPOINT s",
  ];
  for (const sql of refused) {
    const text = errText(await exec(sql));
    // The Wasm build has no load_extension (SQLITE_OMIT_LOAD_EXTENSION).
    if (sql.includes("load_extension")) assert.match(text, /^sql: no such function: load_extension/, sql);
    else assert.equal(text, "sql: not authorized", sql);
  }
  assert.equal(okText(await query("PRAGMA user_version")), '[{"user_version":0}]');
  okText(await exec("PRAGMA foreign_keys = ON"));
  assert.equal(okText(await query("PRAGMA foreign_keys")), '[{"foreign_keys":1}]');
  assert.equal(JSON.parse(okText(await query("PRAGMA table_info(t)")))[0].name, "a");
  okText(await query("PRAGMA index_list(t)"));

  // a_failed_batch_rolls_back
  okText(await exec("CREATE TABLE b (a INTEGER UNIQUE)"));
  assert.deepEqual(await call(2, "", '[{"sql":"INSERT INTO b VALUES (?)","params":[1]},{"sql":"INSERT INTO b VALUES (2)"}]'), [true, 0, "null", null]);
  assert.match(errText(await call(2, "", '[{"sql":"INSERT INTO b VALUES (?)","params":[3]},{"sql":"INSERT INTO b VALUES (?)","params":[1]}]')), /UNIQUE/);
  assert.equal(okText(await query("SELECT a FROM b ORDER BY a")), '[{"a":1},{"a":2}]');
  assert.match(errText(await call(2, "", '{"sql":"x"}')), /JSON array/);
  assert.match(errText(await call(2, "", '[{"params":[]}]')), /JSON array/);
  assert.match(errText(await call(2, "", '[{"sql":"INSERT INTO b VALUES (4)"},{"sql":"COMMIT"}]')), /not authorized/);
  assert.equal(okText(await query("SELECT count(*) AS n FROM b")), '[{"n":2}]');

  // migrate_runs_only_new_steps (on a second app id: its own database)
  const m = (steps) => e.call("com.example.migrate", 3, "", JSON.stringify(steps));
  const v1 = ["CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT)", "INSERT INTO notes (title) VALUES ('first')"];
  assert.deepEqual(await m(v1), [true, 2, "2", null]);
  assert.deepEqual(await m(v1), [true, 2, "2", null]);
  const v2 = [...v1, "ALTER TABLE notes ADD COLUMN done INTEGER NOT NULL DEFAULT 0"];
  assert.deepEqual(await m(v2), [true, 3, "3", null]);
  assert.equal(okText(await e.call("com.example.migrate", 1, "SELECT title, done FROM notes", "")), '[{"title":"first","done":0}]');
  assert.match(errText(await m(v1)), /append-only/);
  assert.match(errText(await m([...v2, "CREATE TABLE extra (a)", "NOT SQL"])), /^sql: /);
  assert.equal(okText(await e.call("com.example.migrate", 1, "PRAGMA user_version", "")), '[{"user_version":3}]');
  assert.match(errText(await e.call("com.example.migrate", 1, "SELECT * FROM extra", "")), /no such table/);
  assert.match(errText(await e.call("com.example.migrate", 3, "", '["a", 1]')), /JSON array of strings/);

  // two_stores_do_not_see_each_other (app ids; the same id in other case is the same app, as files)
  assert.match(errText(await e.call("com.example.other", 1, "SELECT * FROM notes", "")), /no such table: notes/);
  okText(await e.call("COM.EXAMPLE.MIGRATE", 1, "SELECT * FROM notes", ""));

  // fts5_works
  okText(await exec("CREATE VIRTUAL TABLE docs USING fts5(title, body)"));
  okText(await exec("INSERT INTO docs (title, body) VALUES (?, ?)", '["Groceries", "buy milk and bread"]'));
  okText(await exec("INSERT INTO docs (title, body) VALUES (?, ?)", '["Work", "call the plumber"]'));
  assert.equal(okText(await query("SELECT title FROM docs WHERE docs MATCH ? ORDER BY rank", '["milk"]')), '[{"title":"Groceries"}]');

  // temporary data stays in memory
  const db = await e.database("com.example.sql");
  db.internalOn = true;
  assert.equal(String(db.internalValue("PRAGMA temp_store")), "2", "temp_store is MEMORY");
  db.internalOn = false;
  okText(await exec("CREATE TEMP TABLE tmp (x)"));
  okText(await exec("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 20000) INSERT INTO tmp SELECT i FROM n"));
  assert.equal(okText(await query("SELECT x FROM tmp ORDER BY x DESC LIMIT 1")), '[{"x":20000}]');
}

/** The limits: time, quota, result size and memory (each on a new engine). */
async function limits() {
  const forever = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) AS n FROM c";
  const t = await engine({ statementTimeoutMs: 100 });
  const start = Date.now();
  assert.equal(errText(await t.call("a", 1, forever, "")), TOO_LONG);
  assert.ok(Date.now() - start < 5000);
  assert.equal(errText(await t.call("a", 0, forever, "")), TOO_LONG);
  assert.equal(okText(await t.call("a", 1, "SELECT 1 AS one", "")), '[{"one":1}]');

  const q = await engine({ quota: 64 << 10 });
  okText(await q.call("a", 0, "CREATE TABLE t (s TEXT)", ""));
  const big = JSON.stringify(["x".repeat(8 << 10)]);
  let error = null;
  for (let i = 0; i < 100 && !error; i++) {
    const r = await q.call("a", 0, "INSERT INTO t VALUES (?)", big);
    if (!r[0]) error = r[3];
  }
  assert.equal(error, FULL);
  okText(await q.call("a", 1, "SELECT count(*) AS n FROM t", ""));
  okText(await q.call("a", 0, "DELETE FROM t", ""));

  const res = await engine({ maxResult: 1000 });
  const rows = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?) SELECT x FROM c";
  okText(await res.call("a", 1, rows, "[10]"));
  assert.equal(errText(await res.call("a", 1, rows, "[1000]")), TOO_LARGE);

  // The heap limit: a small Wasm memory, and a temporary table that does not fit.
  const mem = await engine({}, 32 << 20);
  okText(await mem.call("a", 0, "CREATE TEMP TABLE t (s)", ""));
  const fill = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 200000) INSERT INTO t SELECT randomblob(1000) FROM n";
  assert.equal(errText(await mem.call("a", 0, fill, "")), OUT_OF_MEMORY);
  assert.equal(okText(await mem.call("a", 1, "SELECT 1 AS one", "")), '[{"one":1}]');
}

/** The Worker protocol (`serveSql` and `SqlWorkerHost`) over a `MessageChannel`. */
async function workerProtocol() {
  const { port1, port2 } = new MessageChannel();
  serveSql(port2, () => engine());
  const fakeWorker = () => {
    const w = {
      postMessage: (m) => port1.postMessage(m),
      set onmessage(f) {
        port1.onmessage = f;
      },
    };
    return w;
  };
  const host = new SqlWorkerHost("unused", fakeWorker);
  const space = host.space("com.example.worker");
  const answers = await Promise.all([
    space.call(3, "", '["CREATE TABLE t (a)"]'),
    space.call(0, "INSERT INTO t VALUES (?)", "[7]"),
    space.call(2, "", '[{"sql":"INSERT INTO t VALUES (8)"}]'),
    space.call(1, "SELECT a FROM t ORDER BY a", ""),
    space.call(0, "NOT SQL", ""),
  ]);
  assert.deepEqual(answers.slice(0, 4), [
    [true, 1, "1", null],
    [true, 1, '{"changes":1,"lastInsertId":1}', null],
    [true, 0, "null", null],
    [true, 0, '[{"a":7},{"a":8}]', null],
  ]);
  assert.match(errText(answers[4]), /^sql: /);
  assert.equal(host.inMemory, true);
  port1.close();

  // An engine that does not start: every call answers an error.
  const ch = new MessageChannel();
  serveSql(ch.port2, () => Promise.reject(new Error("no OPFS lock")));
  const bad = new SqlWorkerHost("unused", () => ({ postMessage: (m) => ch.port1.postMessage(m), set onmessage(f) { ch.port1.onmessage = f; } }));
  assert.equal(errText(await bad.space("x").call(1, "SELECT 1", "")), "sql: the database engine did not start: no OPFS lock");
  ch.port1.close();
}

function bridgeMessages() {
  const m = (fields) => checkFrameMessage({ channel: CHANNEL, ...fields });
  assert.ok(m({ type: "sql-run", id: 1, kind: 0, statement: "SELECT 1", params: "[]" }));
  assert.ok(m({ type: "sql-run", id: 1, kind: 3, statement: "", params: '["CREATE TABLE a (x)"]' }));
  assert.equal(m({ type: "sql-run", id: 1, kind: "0", statement: "", params: "" }), null, "kind must be a number");
  assert.equal(m({ type: "sql-run", id: 1, kind: 1.5, statement: "", params: "" }), null, "kind must be an integer");
  assert.equal(m({ type: "sql-run", id: 1, kind: 256, statement: "", params: "" }), null, "kind is a u8");
  assert.equal(m({ type: "sql-run", id: 1, kind: 0, params: "" }), null, "statement is required");
  assert.equal(m({ type: "sql-run", id: 1, kind: 0, statement: "", params: null }), null, "params must be a string");
  assert.equal(m({ type: "sql-run", id: 1, kind: 0, statement: "x".repeat((16 << 20) + 1), params: "" }), null, "statement too long");
}

// -- The shared app cases (crates/plinth-compiler/tests/sql_cases.txt) ------

/** Reads the shared case file: `{ template, cases: [{ name, decls, body, expect }] }`. */
export function readCases(text) {
  const out = { template: "", cases: [] };
  let current = null;
  let field = null;
  for (const line of text.replace(/\r\n/g, "\n").split("\n")) {
    const head = line.match(/^=== (template|case (\S+))$/);
    if (head) {
      current = head[1] === "template" ? { template: true, text: [] } : { name: head[2], decls: [], body: [], expect: [] };
      field = current.template ? "text" : null;
      if (current.template) out.templateLines = current.text;
      else out.cases.push(current);
      continue;
    }
    const sub = line.match(/^--- (decls|body|expect)$/);
    if (sub && current && !current.template) {
      field = sub[1];
      continue;
    }
    if (current && field) current[field].push(line);
    else if (line.trim() !== "" && !line.startsWith("#")) throw new Error(`sql_cases.txt: a line outside a section: ${line}`);
  }
  const join = (lines) => {
    while (lines.length && lines[lines.length - 1] === "") lines.pop();
    return lines.join("\n");
  };
  out.template = join(out.templateLines ?? []);
  out.cases = out.cases.map((c) => ({ name: c.name, decls: join(c.decls), body: join(c.body), expect: join(c.expect) }));
  return out;
}

/** Finds the CLI binary. */
function cliPath() {
  for (const name of ["plinth.exe", "plinth"]) {
    const p = path.join(root, "target/debug", name);
    if (existsSync(p)) return p;
  }
  throw new Error("build the CLI first: cargo build -p plinth-cli");
}

/** Builds one case into a `.plnt` with the CLI and gives its bytes. */
function buildCase(dir, source, capabilities) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(path.join(dir, "app"), { recursive: true });
  const caps = capabilities.map((name) => `\n[[capabilities]]\nname = "${name}"\nrationale = "Test: web/test/run-sql.mjs."\n`).join("");
  writeFileSync(path.join(dir, "plinth.toml"), `id = "dev.plinth.test.sql-case"\nname = "SQL Case"\nversion = "0.1.0"\npublisher = "plinth"\n${caps}`);
  writeFileSync(path.join(dir, "app/main.tsx"), source);
  try {
    execFileSync(cliPath(), ["build", dir], { stdio: ["ignore", "ignore", "pipe"], encoding: "utf-8" });
  } catch (err) {
    throw new Error(`plinth build ${path.basename(dir)} failed:\n${err.stderr ?? err}`);
  }
  const file = readdirSync(path.join(dir, "dist")).find((f) => f.endsWith(".plnt"));
  return new Uint8Array(readFileSync(path.join(dir, "dist", file)));
}

/**
 * Runs one built app: presses its only button and waits for `done:…`, as
 * `App::go` in the Rust test. `sql`: the backend of `store.sql`.
 */
async function runApp(coreBytes, plnt, { sql = null, refused = new Set(), manifestText: manifest = null, viaWorker = false } = {}) {
  const { appWasm, manifestText: built } = await readPlnt(plnt);
  const manifestText = manifest ?? built;
  let app;
  let channel = null;
  if (viaWorker) {
    // The Worker split of a browser (app-worker.js): the app's `sql.run`
    // crosses to the document as an `sql-run` request, over a MessageChannel.
    channel = new MessageChannel();
    runWorker(channel.port1);
    app = new WorkerApp(channel.port2);
  } else app = new PlinthApp();
  let text = "";
  let button = null;
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") text = String(op.value);
      else if (op.op === "listen" && op.event === 1) button = op.handler;
    }
  };
  const errors = [];
  await app.load(coreBytes, appWasm, { log: () => {}, reportError: (s) => errors.push(s), manifestText, sql, refused });
  await app.init([]);
  assert.ok(button !== null, "the app has a button");
  app.onEvent({ kind: "ui", handler: button, event: 1, value: null });
  const start = Date.now();
  while (!text.startsWith("done:")) {
    if (Date.now() - start > 20000) throw new Error(`no result; text is ${JSON.stringify(text)}`);
    await new Promise((r) => setTimeout(r, 2));
  }
  assert.deepEqual(errors, [], "uncaught errors");
  if (channel) {
    app.terminate?.();
    channel.port1.close();
    channel.port2.close();
  }
  return text.slice("done:".length);
}

async function sharedCases() {
  const cases = readCases(readFileSync(path.join(root, "crates/plinth-compiler/tests/sql_cases.txt"), "utf-8"));
  assert.ok(cases.cases.length >= 7, "the shared file has the cases");
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const work = path.join(os.tmpdir(), `plinth-run-sql-${process.pid}`);
  // A function replacement: a body with "$&" or "$1" stays as it is.
  const source = (decls, body) => cases.template.replace("{decls}", () => decls).replace("{body}", () => body);
  try {
    for (const c of cases.cases) {
      const plnt = buildCase(path.join(work, c.name.replace(/_/g, "-")), source(c.decls, c.body), ["store.sql"]);
      const e = await engine();
      const got = await runApp(coreBytes, plnt, { sql: localSqlHost(e).space("dev.plinth.test.sql-case") });
      assert.equal(got, c.expect, `case ${c.name}`);
    }

    // The same case through the Worker split of a browser.
    const first0 = cases.cases[0];
    const viaWorker = buildCase(path.join(work, "worker"), source(first0.decls, first0.body), ["store.sql"]);
    assert.equal(await runApp(coreBytes, viaWorker, { sql: localSqlHost(await engine()).space("w"), viaWorker: true }), first0.expect);
    assert.equal(await runApp(coreBytes, viaWorker, { viaWorker: true }), "err:denied:unsupported");

        // The data stays for the next run of the app (one page, one engine),
    // and the migration does not run again.
    const e = await engine();
    const host = localSqlHost(e);
    const notes = cases.cases.find((c) => c.name === "exec_and_query_round_trip").decls;
    const first = buildCase(path.join(work, "keep-1"), source(notes, 'await sql.migrate(SCHEMA);\n  await sql.exec(INSERT, ["kept", 0]);\n  const rows = await sql.query<Note>("SELECT * FROM notes");\n  return rows[0].title;'), ["store.sql"]);
    assert.equal(await runApp(coreBytes, first, { sql: host.space("dev.plinth.test.sql-case") }), "kept");
    const second = buildCase(path.join(work, "keep-2"), source(notes, 'const v = await sql.migrate(SCHEMA);\n  const rows = await sql.query<Note>("SELECT * FROM notes");\n  return v + ":" + rows.length + ":" + rows[0].title;'), ["store.sql"]);
    assert.equal(await runApp(coreBytes, second, { sql: host.space("dev.plinth.test.sql-case") }), "2:1:kept");
    // Another app id does not see it: a new database at version 0.
    const other = buildCase(path.join(work, "keep-4"), source(notes, 'const v = await sql.migrate(SCHEMA);\n  const rows = await sql.query<Note>("SELECT * FROM notes");\n  return v + ":" + rows.length;'), ["store.sql"]);
    assert.equal(await runApp(coreBytes, other, { sql: host.space("dev.plinth.test.other") }), "2:0");
    assert.equal(await runApp(coreBytes, other, { sql: host.space("dev.plinth.test.sql-case") }), "2:1");

    // Denied calls do not trap: undeclared, refused, and a host with no engine.
    const denyBody = 'let r = "";\n  try { await sql.migrate(["CREATE TABLE a (x INTEGER)"]); r = "ok;"; } catch (e) { r = e.message + ";"; }\n  try { await sql.query<{ x: number }>("SELECT 1 AS x"); r = r + "ok"; } catch (e) { r = r + e.message; }\n  return r;';
    const declared = buildCase(path.join(work, "denied"), source("", denyBody), ["store.sql"]);
    // The compiler needs the capability (PL1007), so the manifest loses it after the build.
    const { manifestText } = await readPlnt(declared);
    const without = manifestText.replace('name = "store.sql"', 'name = "store.kv"');
    assert.notEqual(without, manifestText);
    assert.equal(await runApp(coreBytes, declared, { sql: host.space("x"), manifestText: without }), "denied:undeclared;denied:undeclared");
    assert.equal(await runApp(coreBytes, declared, { sql: host.space("x"), refused: new Set(["store.sql"]) }), "denied:refused;denied:refused");
    assert.equal(await runApp(coreBytes, declared, { sql: null }), "denied:unsupported;denied:unsupported");
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

async function main() {
  reals();
  await engineCases();
  await limits();
  await workerProtocol();
  bridgeMessages();
  await sharedCases();
  console.log("run-sql.mjs: ok");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
