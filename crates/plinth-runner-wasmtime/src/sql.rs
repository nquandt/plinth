//! The desktop engine of `store.sql` (`docs/SQL.md`, step 1 of §7).
//!
//! Each app has one SQLite database: `<app id>+sql/store.sqlite`, next to
//! the app's private space and not in it (`database_dir`). The host
//! derives the path; the app cannot name a file (`docs/SQL.md` §4
//! rule 1). The host opens the file with `SQLITE_OPEN_NOFOLLOW` and without
//! `SQLITE_OPEN_URI`. (The bundled SQLite is compiled with
//! `SQLITE_USE_URI`, so a name that starts with `file:` is a URI. The host
//! always gives an absolute path, which never starts with `file:`, and the
//! authorizer refuses `ATTACH`, the only other way to name a file.)
//!
//! The calls (`docs/SQL.md` §3) take and give JSON text, because the WIT
//! call of step 2 passes `params` as JSON text:
//!
//! | Call | Input | Result text |
//! |---|---|---|
//! | `exec` | a statement, and a JSON array of parameters | `{"changes":n,"lastInsertId":n}` |
//! | `query` | a statement, and a JSON array of parameters | a JSON array of row objects |
//! | `batch` | a JSON array of `{"sql":…,"params":[…]}` | `null` |
//! | `migrate` | a JSON array of statements | the new version, a JSON number |
//!
//! A parameter is a number, a string, a boolean (bound as 0 or 1) or
//! `null`. A column value is a number, a string or `null`; a `BLOB` value
//! is an error (`docs/SQL.md` SQL-Q3).
//!
//! Security and limits (`docs/SQL.md` §4):
//!
//! - An authorizer refuses `ATTACH` and `DETACH` (and thus also `VACUUM`,
//!   which attaches a database internally), `load_extension`, transaction
//!   and savepoint statements (`batch` and `migrate` make the transaction),
//!   and every `PRAGMA` except `user_version` (read only), `foreign_keys`,
//!   `table_info` and `index_list`, and `data_version` (read only; FTS5
//!   reads it in its own statements, and it tells only if the file changed). The host sets `max_page_count` before
//!   the authorizer is on, and `user_version` and its own transactions with
//!   an internal bypass flag.
//! - Extension loading is off (the bundled SQLite turns it on by default).
//!   The connection is in defensive mode and does not trust the schema.
//! - A statement stops after `Limits::statement_timeout` (a progress
//!   handler): `sql: the query took too long`.
//! - The database can be at most `Limits::quota` bytes (`max_page_count`):
//!   `sql: the database is full`.
//! - A `query` result can be at most `Limits::max_result` bytes of JSON.
//!
//! The calls run on one worker thread for each database, in the order the
//! app made them, and never on the UI thread. Each result goes to the
//! completion queue of `plinth:net` and `plinth:files`
//! (`HostState::net_results`), as the same 4-element list:
//! `[ok, int, text, error]`.

use crate::files::Results;
use plinth_protocol::Value;
use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
use rusqlite::limits::Limit;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, ErrorCode, OpenFlags, Statement};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The capability of the SQL store (SPEC.md §11). Step 2 adds it to
/// `plinth_link::capabilities` and checks it.
pub const STORE_SQL: &str = "store.sql";

/// The file name of the database in the app's database folder.
pub const FILE_NAME: &str = "store.sqlite";

/// The database folder of the app whose `plinth:files` private space is
/// `files_root`: the sibling folder `<app id>+sql`. The database is not in
/// the files folder, so `plinth:files` cannot read, change or remove the
/// database or its journal, and the database does not count toward the
/// files quota. No app id gives a folder name with `+`
/// (`files::private_space_dir` hashes such ids), so the name is never the
/// private space of another app.
pub fn database_dir(files_root: &Path) -> PathBuf {
    let mut name = files_root.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push("+sql");
    files_root.with_file_name(name)
}

/// The time limit of one statement: the same as the time limit of an app call.
pub const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(2);
/// The largest database, in bytes.
pub const DEFAULT_QUOTA: u64 = 256 << 20;
/// The largest `query` result, in bytes of JSON text.
pub const DEFAULT_MAX_RESULT: usize = 16 << 20;

/// The error text of a statement that ran past its time limit.
pub const TOO_LONG: &str = "sql: the query took too long";
/// The error text of a write past the quota.
pub const FULL: &str = "sql: the database is full";
/// The error text of a `query` result past its size limit.
pub const TOO_LARGE: &str = "sql: the result is too large";

/// How many SQLite virtual machine steps run between two checks of the
/// time limit.
const PROGRESS_STEPS: i32 = 1000;

/// The limits of one database (`docs/SQL.md` §4). A host policy can change them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub statement_timeout: Duration,
    pub quota: u64,
    pub max_result: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { statement_timeout: DEFAULT_STATEMENT_TIMEOUT, quota: DEFAULT_QUOTA, max_result: DEFAULT_MAX_RESULT }
    }
}

/// One `store.sql` call.
#[derive(Debug, Clone)]
pub enum Op {
    /// One statement; `params` is a JSON array (or empty text for none).
    Exec { sql: String, params: String },
    /// One statement that gives rows; `params` as for `Exec`.
    Query { sql: String, params: String },
    /// A JSON array of `{"sql": "...", "params": [...]}`, in one transaction.
    Batch { steps: String },
    /// A JSON array of statements; runs those after `user_version`.
    Migrate { steps: String },
}

impl Op {
    /// The call for the `kind` of the WIT call `run(kind, statement,
    /// params)` (`docs/SQL.md` §5): 0 `exec`, 1 `query`, 2 `batch`,
    /// 3 `migrate`. For `batch` and `migrate`, `params` is the JSON array of
    /// steps and `statement` is not used. `None` for an unknown kind.
    pub fn from_kind(kind: u8, statement: String, params: String) -> Option<Op> {
        match kind {
            0 => Some(Op::Exec { sql: statement, params }),
            1 => Some(Op::Query { sql: statement, params }),
            2 => Some(Op::Batch { steps: params }),
            3 => Some(Op::Migrate { steps: params }),
            _ => None,
        }
    }
}

/// The result of `exec`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecResult {
    pub changes: u64,
    pub last_insert_id: i64,
}

impl ExecResult {
    fn to_json(self) -> String {
        serde_json::json!({ "changes": self.changes, "lastInsertId": self.last_insert_id }).to_string()
    }
}

/// One open database. All calls are synchronous; `SqlStore` runs them on
/// a worker thread.
pub struct Db {
    conn: Connection,
    limits: Limits,
    /// When the statement that runs now must stop. `None` between statements.
    deadline: Arc<Mutex<Option<Instant>>>,
    /// Set while the host runs its own statements (`user_version`, the
    /// transactions of `batch` and `migrate`); the authorizer then allows all.
    internal: Arc<AtomicBool>,
}

impl Db {
    /// Opens (or makes) `<dir>/store.sqlite`. Makes `dir` if necessary.
    pub fn open(dir: &Path, limits: Limits) -> Result<Db, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("sql: cannot make the folder of the database: {e}"))?;
        let path = dir.join(FILE_NAME);
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_EXRESCODE;
        let conn = Connection::open_with_flags(&path, flags).map_err(|e| sql_error(&e))?;
        let db = Db { conn, limits, deadline: Arc::new(Mutex::new(None)), internal: Arc::new(AtomicBool::new(false)) };
        db.configure().map_err(|e| sql_error(&e))?;
        Ok(db)
    }

    /// The settings before the app's first statement. The authorizer goes on last.
    fn configure(&self) -> rusqlite::Result<()> {
        use rusqlite::config::DbConfig;
        let conn = &self.conn;
        // SAFETY: `handle` is the open connection of `self.conn`, and
        // `sqlite3_enable_load_extension` only sets a flag on it. This turns
        // off both the C function and the SQL function `load_extension`.
        let rc = unsafe { rusqlite::ffi::sqlite3_enable_load_extension(conn.handle(), 0) };
        if rc != rusqlite::ffi::SQLITE_OK {
            return Err(rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rc), Some("cannot turn off extension loading".into())));
        }
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
        conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
        conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;

        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let max_pages = (self.limits.quota / page_size.max(512) as u64).max(1);
        conn.query_row(&format!("PRAGMA max_page_count = {max_pages}"), [], |_| Ok(()))?;

        let deadline = self.deadline.clone();
        conn.progress_handler(
            PROGRESS_STEPS,
            Some(move || match *deadline.lock().unwrap() {
                Some(at) => Instant::now() >= at,
                None => false,
            }),
        )?;

        let internal = self.internal.clone();
        conn.authorizer(Some(move |ctx: AuthContext<'_>| {
            if internal.load(Ordering::SeqCst) {
                return Authorization::Allow;
            }
            authorize(&ctx.action)
        }))?;
        Ok(())
    }

    /// Runs `f` with the time limit on. A statement that runs past it gives
    /// `TOO_LONG`.
    fn timed<T>(&self, f: impl FnOnce() -> rusqlite::Result<T>) -> Result<T, String> {
        *self.deadline.lock().unwrap() = Some(Instant::now() + self.limits.statement_timeout);
        let r = f();
        *self.deadline.lock().unwrap() = None;
        r.map_err(|e| sql_error(&e))
    }

    /// Runs a statement of the host itself, with the authorizer off.
    fn internal(&self, sql: &str) -> rusqlite::Result<()> {
        self.internal.store(true, Ordering::SeqCst);
        let r = self.conn.execute_batch(sql);
        self.internal.store(false, Ordering::SeqCst);
        r
    }

    /// The database's `user_version`.
    pub fn version(&self) -> Result<i64, String> {
        self.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| sql_error(&e))
    }

    /// `exec(statement, params)`.
    pub fn exec(&self, sql: &str, params_json: &str) -> Result<ExecResult, String> {
        let params = parse_params(params_json)?;
        self.exec_values(sql, &params)
    }

    fn exec_values(&self, sql: &str, params: &[serde_json::Value]) -> Result<ExecResult, String> {
        let mut stmt = self.conn.prepare(sql).map_err(|e| sql_error(&e))?;
        bind(&mut stmt, params)?;
        let readonly = stmt.readonly();
        self.timed(|| {
            // Step to the end: a statement with `RETURNING`, or one that
            // gives rows, also runs.
            let mut rows = stmt.raw_query();
            while rows.next()?.is_some() {}
            Ok(())
        })?;
        let changes = if readonly { 0 } else { self.conn.changes() };
        Ok(ExecResult { changes, last_insert_id: self.conn.last_insert_rowid() })
    }

    /// `query(statement, params)`: a JSON array of row objects.
    pub fn query(&self, sql: &str, params_json: &str) -> Result<String, String> {
        let params = parse_params(params_json)?;
        let mut stmt = self.conn.prepare(sql).map_err(|e| sql_error(&e))?;
        bind(&mut stmt, &params)?;
        let names: Vec<String> = stmt.column_names().into_iter().map(str::to_owned).collect();
        let max = self.limits.max_result;
        let mut out = Vec::from(&b"["[..]);
        let mut failure: Option<String> = None;
        self.timed(|| {
            let mut rows = stmt.raw_query();
            let mut first = true;
            while let Some(row) = rows.next()? {
                let mut obj = serde_json::Map::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    let v = match row.get_ref(i)? {
                        ValueRef::Null => serde_json::Value::Null,
                        ValueRef::Integer(n) => serde_json::Value::from(n),
                        // SQLite can hold an infinity, which JSON cannot:
                        // `JSON.stringify` gives `null` for it too.
                        ValueRef::Real(f) => serde_json::Number::from_f64(f).map_or(serde_json::Value::Null, serde_json::Value::Number),
                        ValueRef::Text(t) => serde_json::Value::String(String::from_utf8_lossy(t).into_owned()),
                        ValueRef::Blob(_) => {
                            failure = Some(format!("sql: the column \"{name}\" is a BLOB, which store.sql cannot read"));
                            return Ok(());
                        }
                    };
                    obj.insert(name.clone(), v);
                }
                if !first {
                    out.push(b',');
                }
                first = false;
                serde_json::to_writer(&mut out, &obj).expect("a JSON map writes to a Vec");
                if out.len() + 1 > max {
                    failure = Some(TOO_LARGE.into());
                    return Ok(());
                }
            }
            Ok(())
        })?;
        if let Some(e) = failure {
            return Err(e);
        }
        out.push(b']');
        Ok(String::from_utf8(out).expect("serde_json writes UTF-8"))
    }

    /// `batch(steps)`: all steps in one transaction; a failed step rolls
    /// back all of them.
    pub fn batch(&self, steps_json: &str) -> Result<(), String> {
        let steps = parse_batch(steps_json)?;
        self.transaction(|db| {
            for (sql, params) in &steps {
                db.exec_values(sql, params)?;
            }
            Ok(())
        })
    }

    /// `migrate(steps)`: runs the steps after `user_version` in one
    /// transaction, and sets `user_version` to the number of steps. Gives
    /// the new version.
    pub fn migrate(&self, steps_json: &str) -> Result<i64, String> {
        let steps: Vec<String> =
            serde_json::from_str(steps_json).map_err(|_| "sql: the migration steps must be a JSON array of strings".to_owned())?;
        let target = steps.len() as i64;
        let current = self.version()?;
        if current > target {
            return Err(format!(
                "sql: the database is at version {current}, but there are only {target} migration steps (steps are append-only)"
            ));
        }
        if current == target {
            return Ok(current);
        }
        self.transaction(|db| {
            for sql in &steps[current as usize..] {
                db.exec_values(sql, &[])?;
            }
            db.internal(&format!("PRAGMA user_version = {target}")).map_err(|e| sql_error(&e))
        })?;
        Ok(target)
    }

    fn transaction(&self, f: impl FnOnce(&Db) -> Result<(), String>) -> Result<(), String> {
        self.internal("BEGIN IMMEDIATE").map_err(|e| sql_error(&e))?;
        match f(self) {
            Ok(()) => match self.internal("COMMIT") {
                Ok(()) => Ok(()),
                Err(e) => {
                    let _ = self.internal("ROLLBACK");
                    Err(sql_error(&e))
                }
            },
            Err(e) => {
                // SQLite can roll back by itself (after an interrupt or a
                // full disk); then this ROLLBACK fails, which is correct.
                let _ = self.internal("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Runs one call and makes its completion result `[ok, int, text,
    /// error]`. `int` is the number of changes for `exec`, the number of
    /// rows is not given for `query` (0), 0 for `batch`, and the new version
    /// for `migrate`.
    pub fn run(&self, op: Op) -> Value {
        match op {
            Op::Exec { sql, params } => match self.exec(&sql, &params) {
                Ok(r) => success(r.changes as i32, r.to_json()),
                Err(e) => failure(e),
            },
            Op::Query { sql, params } => match self.query(&sql, &params) {
                Ok(text) => success(0, text),
                Err(e) => failure(e),
            },
            Op::Batch { steps } => match self.batch(&steps) {
                Ok(()) => success(0, "null".into()),
                Err(e) => failure(e),
            },
            Op::Migrate { steps } => match self.migrate(&steps) {
                Ok(v) => success(v as i32, v.to_string()),
                Err(e) => failure(e),
            },
        }
    }
}

/// The authorizer's rule for one action of an app statement.
fn authorize(action: &AuthAction<'_>) -> Authorization {
    match action {
        AuthAction::Attach { .. } | AuthAction::Detach { .. } => Authorization::Deny,
        AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => Authorization::Deny,
        AuthAction::Function { function_name } if function_name.eq_ignore_ascii_case("load_extension") => Authorization::Deny,
        AuthAction::Pragma { pragma_name, pragma_value } => {
            let name = pragma_name.to_ascii_lowercase();
            let allowed = match name.as_str() {
                // FTS5 reads `data_version` in its own statements.
                "user_version" | "data_version" => pragma_value.is_none(),
                "foreign_keys" | "table_info" | "index_list" => true,
                _ => false,
            };
            if allowed { Authorization::Allow } else { Authorization::Deny }
        }
        // An action this version of rusqlite does not know.
        AuthAction::Unknown { .. } => Authorization::Deny,
        _ => Authorization::Allow,
    }
}

/// The error text the app sees for a SQLite error.
fn sql_error(e: &rusqlite::Error) -> String {
    if let Some(code) = e.sqlite_error_code() {
        match code {
            ErrorCode::OperationInterrupted => return TOO_LONG.into(),
            ErrorCode::DiskFull => return FULL.into(),
            _ => {}
        }
    }
    match e {
        rusqlite::Error::SqliteFailure(err, _) if err.code == ErrorCode::AuthorizationForStatementDenied => "sql: not authorized".into(),
        rusqlite::Error::SqliteFailure(_, Some(msg)) => format!("sql: {msg}"),
        rusqlite::Error::SqliteFailure(err, None) => format!("sql: {err}"),
        rusqlite::Error::MultipleStatement => "sql: give one statement only".into(),
        other => format!("sql: {other}"),
    }
}

/// Parses a JSON array of parameters. Empty text is no parameters.
fn parse_params(json: &str) -> Result<Vec<serde_json::Value>, String> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(a)) => Ok(a),
        Ok(serde_json::Value::Null) => Ok(Vec::new()),
        _ => Err("sql: the parameters must be a JSON array".into()),
    }
}

/// Parses the steps of `batch`: `[{"sql": "...", "params": [...]}, ...]`.
fn parse_batch(json: &str) -> Result<Vec<(String, Vec<serde_json::Value>)>, String> {
    const SHAPE: &str = "sql: the batch must be a JSON array of {\"sql\": string, \"params\"?: array}";
    let serde_json::Value::Array(items) = serde_json::from_str::<serde_json::Value>(json).map_err(|_| SHAPE.to_owned())? else {
        return Err(SHAPE.into());
    };
    let mut steps = Vec::with_capacity(items.len());
    for item in items {
        let serde_json::Value::Object(mut obj) = item else {
            return Err(SHAPE.into());
        };
        let Some(serde_json::Value::String(sql)) = obj.remove("sql") else {
            return Err(SHAPE.into());
        };
        let params = match obj.remove("params") {
            None | Some(serde_json::Value::Null) => Vec::new(),
            Some(serde_json::Value::Array(a)) => a,
            Some(_) => return Err(SHAPE.into()),
        };
        steps.push((sql, params));
    }
    Ok(steps)
}

/// Binds the parameters to `stmt` (1-based `?` positions).
fn bind(stmt: &mut Statement<'_>, params: &[serde_json::Value]) -> Result<(), String> {
    let expected = stmt.parameter_count();
    if expected != params.len() {
        return Err(format!("sql: the statement has {expected} parameters, but the call gives {}", params.len()));
    }
    for (i, p) in params.iter().enumerate() {
        let at = i + 1;
        let r = match p {
            serde_json::Value::Null => stmt.raw_bind_parameter(at, rusqlite::types::Null),
            serde_json::Value::Bool(b) => stmt.raw_bind_parameter(at, i64::from(*b)),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => stmt.raw_bind_parameter(at, i),
                None => stmt.raw_bind_parameter(at, n.as_f64().unwrap_or(f64::NAN)),
            },
            serde_json::Value::String(s) => stmt.raw_bind_parameter(at, s.as_str()),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                return Err(format!("sql: parameter {at} must be a number, a string, a boolean or null"));
            }
        };
        r.map_err(|e| sql_error(&e))?;
    }
    Ok(())
}

fn success(int: i32, text: String) -> Value {
    Value::List(vec![Value::Bool(true), Value::Int(int), Value::Str(text), Value::Null])
}

fn failure(error: String) -> Value {
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(error)])
}

struct Job {
    id: u32,
    op: Op,
    results: Results,
}

/// One app's SQL store and its worker thread. The worker opens the
/// database at its first call and keeps it open until the store is dropped.
pub struct SqlStore {
    dir: PathBuf,
    limits: Limits,
    worker: Mutex<Option<Sender<Job>>>,
}

impl SqlStore {
    /// A store whose database is `<dir>/store.sqlite` (`dir` is the app's
    /// private space, `files::private_space_dir`).
    pub fn open(dir: PathBuf, limits: Limits) -> Self {
        Self { dir, limits, worker: Mutex::new(None) }
    }

    /// The path of the database file.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// Queues `op` on the worker thread. The result goes to `results`
    /// under `id`. The caller checks the capability first.
    pub fn submit(&self, id: u32, op: Op, results: &Results) {
        let mut worker = self.worker.lock().unwrap();
        if worker.is_none() {
            let (tx, rx) = channel::<Job>();
            let (dir, limits) = (self.dir.clone(), self.limits);
            std::thread::Builder::new()
                .name("plinth-sql".into())
                .spawn(move || {
                    let mut db: Option<Db> = None;
                    for job in rx {
                        if db.is_none() {
                            match Db::open(&dir, limits) {
                                Ok(d) => db = Some(d),
                                Err(e) => {
                                    job.results.lock().unwrap().push((job.id, failure(e)));
                                    continue;
                                }
                            }
                        }
                        let result = db.as_ref().expect("opened above").run(job.op);
                        job.results.lock().unwrap().push((job.id, result));
                    }
                })
                .expect("start the sql worker");
            *worker = Some(tx);
        }
        let job = Job { id, op, results: results.clone() };
        if let Err(e) = worker.as_ref().expect("made above").send(job) {
            let job = e.0;
            job.results.lock().unwrap().push((job.id, failure("sql: the database worker stopped".into())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plinth-sql-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn open(name: &str) -> (Db, PathBuf) {
        open_with(name, Limits::default())
    }

    fn open_with(name: &str, limits: Limits) -> (Db, PathBuf) {
        let dir = temp_dir(name);
        (Db::open(&dir, limits).unwrap(), dir)
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn exec_and_query_round_trip() {
        let (db, dir) = open("roundtrip");
        db.exec("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, score REAL)", "")
            .unwrap();
        let r = db.exec("INSERT INTO notes (title, score) VALUES (?, ?)", r#"["Buy milk", 1.5]"#).unwrap();
        assert_eq!(r, ExecResult { changes: 1, last_insert_id: 1 });
        assert_eq!(r.to_json(), r#"{"changes":1,"lastInsertId":1}"#);
        let r = db.exec("INSERT INTO notes (title, done) VALUES (?, ?)", r#"["Call Bob", true]"#).unwrap();
        assert_eq!(r.last_insert_id, 2);

        let rows = db.query("SELECT id, title, done, score FROM notes ORDER BY id", "[]").unwrap();
        assert_eq!(
            json(&rows),
            json(r#"[{"id":1,"title":"Buy milk","done":0,"score":1.5},{"id":2,"title":"Call Bob","done":1,"score":null}]"#)
        );
        let rows = db.query("SELECT title FROM notes WHERE done = ?", "[false]").unwrap();
        assert_eq!(json(&rows), json(r#"[{"title":"Buy milk"}]"#));
        assert_eq!(db.query("SELECT * FROM notes WHERE id > 99", "").unwrap(), "[]");

        let r = db.exec("UPDATE notes SET done = 1", "").unwrap();
        assert_eq!(r.changes, 2);
        // A statement that gives rows also runs in `exec`, with no changes.
        assert_eq!(db.exec("SELECT 1", "").unwrap().changes, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn params_of_each_type() {
        let (db, dir) = open("params");
        let rows = db.query("SELECT ? AS i, ? AS f, ? AS s, ? AS t, ? AS b, ? AS n", r#"[42, -2.25, "héllo", true, false, null]"#).unwrap();
        assert_eq!(json(&rows), json(r#"[{"i":42,"f":-2.25,"s":"héllo","t":1,"b":0,"n":null}]"#));

        for bad in [r#"[[1]]"#, r#"[{"a":1}]"#] {
            let e = db.query("SELECT ?", bad).unwrap_err();
            assert!(e.contains("must be a number, a string, a boolean or null"), "{e}");
        }
        assert!(db.query("SELECT ?", r#"{"a":1}"#).unwrap_err().contains("JSON array"));
        assert!(db.query("SELECT ?", "[1, 2]").unwrap_err().contains("has 1 parameters"));
        assert!(db.query("SELECT ?, ?", "[1]").unwrap_err().contains("has 2 parameters"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_are_sql_errors() {
        let (db, dir) = open("errors");
        let e = db.exec("CREATE TABLEX t (a)", "").unwrap_err();
        assert!(e.starts_with("sql: "), "{e}");
        db.exec("CREATE TABLE t (a INTEGER UNIQUE)", "").unwrap();
        db.exec("INSERT INTO t VALUES (1)", "").unwrap();
        let e = db.exec("INSERT INTO t VALUES (1)", "").unwrap_err();
        assert!(e.starts_with("sql: ") && e.contains("UNIQUE"), "{e}");
        let e = db.exec("INSERT INTO t VALUES (2); INSERT INTO t VALUES (3)", "").unwrap_err();
        assert_eq!(e, "sql: give one statement only");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_columns_are_an_error() {
        let (db, dir) = open("blob");
        let e = db.query("SELECT x'00ff' AS data", "").unwrap_err();
        assert!(e.contains("\"data\" is a BLOB"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn authorizer_refuses_the_ways_out() {
        let (db, dir) = open("auth");
        db.exec("CREATE TABLE t (a)", "").unwrap();
        let other = dir.join("other.sqlite").display().to_string().replace('\'', "''");
        let refused = [
            format!("ATTACH '{other}' AS o"),
            "DETACH main".to_owned(),
            "PRAGMA writable_schema = ON".to_owned(),
            "PRAGMA writable_schema".to_owned(),
            "PRAGMA user_version = 5".to_owned(),
            "PRAGMA max_page_count = 100000000".to_owned(),
            "PRAGMA journal_mode = OFF".to_owned(),
            format!("VACUUM INTO '{other}'"),
            "VACUUM".to_owned(),
            "SELECT load_extension('x')".to_owned(),
            "BEGIN".to_owned(),
            "COMMIT".to_owned(),
            "SAVEPOINT s".to_owned(),
        ];
        for sql in &refused {
            let e = db.exec(sql, "").unwrap_err();
            assert!(e.starts_with("sql: not authorized"), "{sql}: {e}");
        }
        assert!(!dir.join("other.sqlite").exists());

        // The safe list.
        assert_eq!(json(&db.query("PRAGMA user_version", "").unwrap()), json(r#"[{"user_version":0}]"#));
        db.exec("PRAGMA foreign_keys = ON", "").unwrap();
        assert_eq!(json(&db.query("PRAGMA foreign_keys", "").unwrap()), json(r#"[{"foreign_keys":1}]"#));
        let info = json(&db.query("PRAGMA table_info(t)", "").unwrap());
        assert_eq!(info[0]["name"], "a");
        db.query("PRAGMA index_list(t)", "").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extension_loading_is_off() {
        let (db, dir) = open("ext");
        // Also with the authorizer off, the SQL function is not available.
        db.internal.store(true, Ordering::SeqCst);
        let e = db.query("SELECT load_extension('nothing')", "").unwrap_err();
        db.internal.store(false, Ordering::SeqCst);
        assert!(e.contains("not authorized"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_statement_that_never_ends_stops() {
        let limits = Limits { statement_timeout: Duration::from_millis(100), ..Limits::default() };
        let (db, dir) = open_with("timeout", limits);
        let forever = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT count(*) AS n FROM c";
        let start = Instant::now();
        assert_eq!(db.query(forever, "").unwrap_err(), TOO_LONG);
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(db.exec(forever, "").unwrap_err(), TOO_LONG);
        // The database still works.
        assert_eq!(json(&db.query("SELECT 1 AS one", "").unwrap()), json(r#"[{"one":1}]"#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_write_past_the_quota_fails() {
        let limits = Limits { quota: 64 << 10, ..Limits::default() };
        let (db, dir) = open_with("quota", limits);
        db.exec("CREATE TABLE t (s TEXT)", "").unwrap();
        let big = format!("[\"{}\"]", "x".repeat(8 << 10));
        let mut error = None;
        for _ in 0..100 {
            if let Err(e) = db.exec("INSERT INTO t VALUES (?)", &big) {
                error = Some(e);
                break;
            }
        }
        assert_eq!(error.as_deref(), Some(FULL));
        let size = std::fs::metadata(dir.join(FILE_NAME)).unwrap().len();
        assert!(size <= 64 << 10, "{size}");
        // Reads and deletes still work.
        db.query("SELECT count(*) AS n FROM t", "").unwrap();
        db.exec("DELETE FROM t", "").unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_result_past_the_limit_fails() {
        let limits = Limits { max_result: 1000, ..Limits::default() };
        let (db, dir) = open_with("result", limits);
        let rows = "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < ?) SELECT x FROM c";
        assert!(db.query(rows, "[10]").is_ok());
        assert_eq!(db.query(rows, "[1000]").unwrap_err(), TOO_LARGE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_batch_rolls_back() {
        let (db, dir) = open("batch");
        db.exec("CREATE TABLE t (a INTEGER UNIQUE)", "").unwrap();
        db.batch(r#"[{"sql":"INSERT INTO t VALUES (?)","params":[1]},{"sql":"INSERT INTO t VALUES (2)"}]"#).unwrap();
        let e =
            db.batch(r#"[{"sql":"INSERT INTO t VALUES (?)","params":[3]},{"sql":"INSERT INTO t VALUES (?)","params":[1]}]"#).unwrap_err();
        assert!(e.contains("UNIQUE"), "{e}");
        assert_eq!(json(&db.query("SELECT a FROM t ORDER BY a", "").unwrap()), json(r#"[{"a":1},{"a":2}]"#));
        assert!(db.batch(r#"{"sql":"x"}"#).unwrap_err().contains("JSON array"));
        assert!(db.batch(r#"[{"params":[]}]"#).unwrap_err().contains("JSON array"));
        // A transaction statement inside a batch is refused.
        let e = db.batch(r#"[{"sql":"INSERT INTO t VALUES (4)"},{"sql":"COMMIT"}]"#).unwrap_err();
        assert!(e.contains("not authorized"), "{e}");
        assert_eq!(json(&db.query("SELECT count(*) AS n FROM t", "").unwrap()), json(r#"[{"n":2}]"#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_runs_only_new_steps() {
        let (db, dir) = open("migrate");
        let v1 = r#"["CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT)", "INSERT INTO notes (title) VALUES ('first')"]"#;
        assert_eq!(db.migrate(v1).unwrap(), 2);
        // The same steps again do nothing (the INSERT does not run again).
        assert_eq!(db.migrate(v1).unwrap(), 2);
        let v2 = r#"["CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT)", "INSERT INTO notes (title) VALUES ('first')", "ALTER TABLE notes ADD COLUMN done INTEGER NOT NULL DEFAULT 0"]"#;
        assert_eq!(db.migrate(v2).unwrap(), 3);
        assert_eq!(db.version().unwrap(), 3);
        assert_eq!(json(&db.query("SELECT title, done FROM notes", "").unwrap()), json(r#"[{"title":"first","done":0}]"#));
        // Fewer steps than the version: an error.
        assert!(db.migrate(v1).unwrap_err().contains("append-only"));
        // A failed step rolls back the whole migration.
        let v3 = r#"["CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT)", "INSERT INTO notes (title) VALUES ('first')", "ALTER TABLE notes ADD COLUMN done INTEGER NOT NULL DEFAULT 0", "CREATE TABLE extra (a)", "NOT SQL"]"#;
        assert!(db.migrate(v3).unwrap_err().starts_with("sql: "));
        assert_eq!(db.version().unwrap(), 3);
        assert!(db.query("SELECT * FROM extra", "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fts5_works() {
        let (db, dir) = open("fts5");
        db.exec("CREATE VIRTUAL TABLE docs USING fts5(title, body)", "").unwrap();
        db.exec("INSERT INTO docs (title, body) VALUES (?, ?)", r#"["Groceries", "buy milk and bread"]"#).unwrap();
        db.exec("INSERT INTO docs (title, body) VALUES (?, ?)", r#"["Work", "call the plumber"]"#).unwrap();
        let rows = db.query("SELECT title FROM docs WHERE docs MATCH ? ORDER BY rank", r#"["milk"]"#).unwrap();
        assert_eq!(json(&rows), json(r#"[{"title":"Groceries"}]"#));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_database_is_not_in_the_files_space() {
        use crate::files::{Owner, private_space_dir};
        let data = Path::new("data");
        let space = private_space_dir(data, &Owner::Dev, "com.example.notes");
        let db = database_dir(&space);
        assert_eq!(db, space.with_file_name("com.example.notes+sql"));
        assert!(!db.starts_with(&space), "plinth:files must not reach the database");
        // No app id maps to the database folder of another app.
        for id in ["com.example.notes+sql", "COM.EXAMPLE.NOTES+SQL"] {
            assert_ne!(private_space_dir(data, &Owner::Dev, id), db);
        }
    }

    #[test]
    fn two_stores_do_not_see_each_other() {
        let (a, dir_a) = open("iso-a");
        let (b, dir_b) = open("iso-b");
        a.exec("CREATE TABLE secret (s TEXT)", "").unwrap();
        a.exec("INSERT INTO secret VALUES ('a')", "").unwrap();
        assert!(b.query("SELECT * FROM secret", "").unwrap_err().contains("no such table"));
        drop(a);
        // The data stays in the file of `a`.
        let a = Db::open(&dir_a, Limits::default()).unwrap();
        assert_eq!(json(&a.query("SELECT s FROM secret", "").unwrap()), json(r#"[{"s":"a"}]"#));
        let _ = std::fs::remove_dir_all(&dir_a);
        let _ = std::fs::remove_dir_all(&dir_b);
    }

    #[test]
    fn the_worker_answers_with_the_request_id() {
        let dir = temp_dir("worker");
        let store = SqlStore::open(dir.clone(), Limits::default());
        assert_eq!(store.path(), dir.join(FILE_NAME));
        let results: Results = Arc::new(Mutex::new(Vec::new()));
        let ops = [
            Op::from_kind(3, String::new(), r#"["CREATE TABLE t (a)"]"#.into()).unwrap(),
            Op::from_kind(0, "INSERT INTO t VALUES (?)".into(), "[7]".into()).unwrap(),
            Op::from_kind(2, String::new(), r#"[{"sql":"INSERT INTO t VALUES (8)"}]"#.into()).unwrap(),
            Op::from_kind(1, "SELECT a FROM t ORDER BY a".into(), String::new()).unwrap(),
            Op::from_kind(0, "NOT SQL".into(), String::new()).unwrap(),
        ];
        assert!(Op::from_kind(9, String::new(), String::new()).is_none());
        for (i, op) in ops.into_iter().enumerate() {
            store.submit(10 + i as u32, op, &results);
        }
        let start = Instant::now();
        while results.lock().unwrap().len() < 5 {
            assert!(start.elapsed() < Duration::from_secs(10), "the worker did not answer");
            std::thread::sleep(Duration::from_millis(5));
        }
        let got = results.lock().unwrap().clone();
        let s = |t: &str| Value::Str(t.into());
        assert_eq!(got[0], (10, Value::List(vec![Value::Bool(true), Value::Int(1), s("1"), Value::Null])));
        assert_eq!(got[1], (11, Value::List(vec![Value::Bool(true), Value::Int(1), s(r#"{"changes":1,"lastInsertId":1}"#), Value::Null])));
        assert_eq!(got[2], (12, Value::List(vec![Value::Bool(true), Value::Int(0), s("null"), Value::Null])));
        assert_eq!(got[3], (13, Value::List(vec![Value::Bool(true), Value::Int(0), s(r#"[{"a":7},{"a":8}]"#), Value::Null])));
        let Value::List(fail) = &got[4].1 else { panic!() };
        assert_eq!(got[4].0, 14);
        assert_eq!(fail[0], Value::Bool(false));
        assert!(matches!(&fail[3], Value::Str(e) if e.starts_with("sql: ")));
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
