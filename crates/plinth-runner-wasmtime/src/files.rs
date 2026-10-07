//! `plinth:files` on the app's private space (core 1.11, `docs/STORAGE.md`
//! §2 item 3, §3; capability `files.private`).
//!
//! The provider is the local file system: one folder for each app identity
//! under the host's data directory (`private_space_dir`). The app gives only
//! a relative path; `check_path` applies rule 1 of `docs/STORAGE.md` §3 here,
//! in the host (the guest's own check is not trusted). Then the host adds
//! the root of the space, one checked segment at a time.
//!
//! The calls run on one worker thread for each app, in the order the app
//! made them (a `write` and then a `read` of the same path see the write),
//! and never on the UI thread. Each result goes to the same completion
//! queue that `plinth:net` uses (`HostState::net_results`), as the same
//! 4-element list: `[ok, 0, text, error]`.
//!
//! Files are UTF-8 text. A file can be at most `MAX_FILE` bytes, and all the
//! files of one app at most its quota (`DEFAULT_QUOTA`, 50 MiB). Directories
//! exist only as parents of files: `write` makes them, and `remove` of the
//! last file in a directory also removes the empty directories above it (the
//! web host has the same model, so both hosts list the same entries).

use crate::policy::{DeniedReason, Policy};
use plinth_protocol::Value;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};

/// The capability of the private space.
pub use plinth_link::capabilities::FILES_PRIVATE;

/// The quota of one app's private space, in bytes of file content.
pub const DEFAULT_QUOTA: u64 = 50 << 20;
/// The largest file that `read` and `write` accept, in bytes.
pub const MAX_FILE: u64 = 8 << 20;
/// The longest path, in UTF-8 bytes.
pub const MAX_PATH: usize = 1024;
/// The longest segment (one file or directory name), in UTF-8 bytes.
pub const MAX_SEGMENT: usize = 255;
/// The most segments in one path.
pub const MAX_DEPTH: usize = 32;

/// The completion queue that a guest's host state drains (shared with
/// `plinth:net`).
pub type Results = Arc<Mutex<Vec<(u32, Value)>>>;

/// Why a path is refused (`docs/STORAGE.md` §3 rule 1). The text after
/// `invalid-path: ` in the error the app sees.
pub fn check_path(path: &str, allow_root: bool) -> Result<Vec<&str>, &'static str> {
    if path.is_empty() {
        return if allow_root { Ok(Vec::new()) } else { Err("empty") };
    }
    if path.len() > MAX_PATH {
        return Err("too long");
    }
    if path.starts_with('/') {
        return Err("absolute");
    }
    if path.contains('\\') {
        return Err("backslash");
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_DEPTH {
        return Err("too deep");
    }
    for seg in &segments {
        check_segment(seg)?;
    }
    Ok(segments)
}

/// The rules for one segment. The same rules on every host, also where a
/// file system would accept the name (so a space can move between hosts):
/// no empty segment, no `.` or `..`, no control character (NUL included),
/// none of `: * ? " < > |` (drive letters, Windows streams and wildcards),
/// no trailing dot or space, and no Windows device name (`CON`, `NUL`,
/// `COM1`, …, with or without an extension).
fn check_segment(seg: &str) -> Result<(), &'static str> {
    if seg.is_empty() {
        return Err("empty segment");
    }
    if seg == "." || seg == ".." {
        return Err("dot segment");
    }
    if seg.len() > MAX_SEGMENT {
        return Err("segment too long");
    }
    if seg.chars().any(|c| c.is_control()) {
        return Err("control character");
    }
    if seg.chars().any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err("reserved character");
    }
    if seg.ends_with('.') || seg.ends_with(' ') {
        return Err("trailing dot or space");
    }
    let stem = seg.split('.').next().unwrap_or(seg).trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if device {
        return Err("device name");
    }
    Ok(())
}

/// Who owns a private space (`docs/STORAGE.md` §3 rule 2): the publisher key
/// of a signed package, else the digest of the package. `Dev` is a project
/// that `plinth dev` runs from source: it has no package yet, so its space
/// is keyed by the app id alone, apart from every installed app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The signer's key id (`ed25519:…`).
    Publisher(String),
    /// The SHA-256 of the `.plnt` bytes, as hex.
    Package(String),
    Dev,
}

impl Owner {
    /// The owner of a package: its verified signer, else its digest.
    pub fn of_package(signer: Option<&str>, package_bytes: &[u8]) -> Owner {
        match signer {
            Some(key) if !key.is_empty() => Owner::Publisher(key.to_owned()),
            _ => Owner::Package(hex(&Sha256::digest(package_bytes))),
        }
    }

    fn segment(&self) -> String {
        match self {
            Owner::Publisher(key) => format!("key-{}", &hex(&Sha256::digest(key.as_bytes()))[..32]),
            Owner::Package(digest) => format!("pkg-{}", &digest[..digest.len().min(32)]),
            Owner::Dev => "dev".into(),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The folder of the private space of `app_id` owned by `owner`:
/// `<data dir>/spaces/private/<owner>/<app id>/`. The app id is lowercased
/// (the web host does the same, and Windows and macOS file names ignore
/// case); an id with other characters than `a-z 0-9 . _ -` is hashed.
pub fn private_space_dir(data_dir: &Path, owner: &Owner, app_id: &str) -> PathBuf {
    let id = app_id.to_ascii_lowercase();
    let plain = !id.is_empty()
        && id.len() <= 128
        && id != "."
        && id != ".."
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
    let id = if plain { id } else { format!("id-{}", &hex(&Sha256::digest(app_id.as_bytes()))[..32]) };
    data_dir.join("spaces").join("private").join(owner.segment()).join(id)
}

/// One app's private space and its worker thread.
pub struct Files {
    root: Option<PathBuf>,
    quota: u64,
    worker: Mutex<Option<Sender<Job>>>,
}

struct Job {
    id: u32,
    op: Op,
    results: Results,
}

/// One `plinth:files` call.
#[derive(Debug)]
pub enum Op {
    Read(String),
    Write(String, String),
    List(String),
    Stat(String),
    Remove(String),
}

impl Files {
    /// A space at `root` (made at the first write) with `quota` bytes.
    pub fn open(root: PathBuf, quota: u64) -> Self {
        Self { root: Some(root), quota, worker: Mutex::new(None) }
    }

    /// No space: every call is `denied:unsupported` (a host with no data
    /// directory, and `Runner::load`).
    pub fn unavailable() -> Self {
        Self { root: None, quota: 0, worker: Mutex::new(None) }
    }

    /// The root folder, if any.
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// Checks the capability, then queues `op` on the worker thread. The
    /// result goes to `results` under `id`; a denied call answers at once.
    pub fn submit(&self, policy: &Policy, id: u32, op: Op, results: &Results) {
        let denied = match policy.check(FILES_PRIVATE) {
            Err(r) => Some(r),
            Ok(()) if self.root.is_none() => Some(DeniedReason::Unsupported),
            Ok(()) => None,
        };
        if let Some(reason) = denied {
            results.lock().unwrap().push((id, failure(denied_text(reason))));
            return;
        }
        let root = self.root.clone().expect("checked above");
        let quota = self.quota;
        let mut worker = self.worker.lock().unwrap();
        if worker.is_none() {
            let (tx, rx) = channel::<Job>();
            std::thread::Builder::new()
                .name("plinth-files".into())
                .spawn(move || {
                    for job in rx {
                        let result = run(&root, quota, job.op);
                        job.results.lock().unwrap().push((job.id, result));
                    }
                })
                .expect("start the files worker");
            *worker = Some(tx);
        }
        let job = Job { id, op, results: results.clone() };
        if let Err(e) = worker.as_ref().expect("made above").send(job) {
            let job = e.0;
            job.results.lock().unwrap().push((job.id, failure("io: the files worker stopped".into())));
        }
    }

    /// Runs `op` on this thread (for tests of the provider).
    pub fn run_now(&self, op: Op) -> Value {
        match &self.root {
            Some(root) => run(root, self.quota, op),
            None => failure(denied_text(DeniedReason::Unsupported)),
        }
    }
}

fn denied_text(reason: DeniedReason) -> String {
    match reason {
        DeniedReason::Undeclared => "denied:undeclared",
        DeniedReason::Refused => "denied:refused",
        DeniedReason::Unsupported => "denied:unsupported",
    }
    .into()
}

fn success(text: String) -> Value {
    Value::List(vec![Value::Bool(true), Value::Int(0), Value::Str(text), Value::Null])
}

fn failure(error: String) -> Value {
    Value::List(vec![Value::Bool(false), Value::Int(0), Value::Str(String::new()), Value::Str(error)])
}

fn run(root: &Path, quota: u64, op: Op) -> Value {
    match run_op(root, quota, op) {
        Ok(text) => success(text),
        Err(e) => failure(e),
    }
}

fn resolve(root: &Path, segments: &[String]) -> PathBuf {
    let mut p = root.to_path_buf();
    for s in segments {
        p.push(s);
    }
    p
}

fn io_error(e: std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => "not-found".into(),
        _ => format!("io: {e}"),
    }
}

/// `{"name":…,"kind":…,"size":…}` for one entry.
fn entry_json(name: &str, meta: &std::fs::Metadata) -> serde_json::Value {
    let (kind, size) = if meta.is_dir() { ("dir", 0) } else { ("file", meta.len()) };
    serde_json::json!({ "name": name, "kind": kind, "size": size })
}

fn run_op(root: &Path, quota: u64, op: Op) -> Result<String, String> {
    let checked = |path: &str, allow_root: bool| -> Result<Vec<String>, String> {
        check_path(path, allow_root).map(|s| s.into_iter().map(str::to_owned).collect()).map_err(|r| format!("invalid-path: {r}"))
    };
    match op {
        Op::Read(path) => {
            let s = checked(&path, false)?;
            let p = resolve(root, &s);
            let meta = std::fs::symlink_metadata(&p).map_err(io_error)?;
            if !meta.is_file() {
                return Err(if meta.is_dir() { "not-a-file".into() } else { "not-found".into() });
            }
            if meta.len() > MAX_FILE {
                return Err("too-large".into());
            }
            let bytes = std::fs::read(&p).map_err(io_error)?;
            String::from_utf8(bytes).map_err(|_| "not-text".to_owned())
        }
        Op::Write(path, text) => {
            let s = checked(&path, false)?;
            if text.len() as u64 > MAX_FILE {
                return Err("too-large".into());
            }
            let p = resolve(root, &s);
            // A parent that is a file, or a directory at `path`.
            let mut dir = root.to_path_buf();
            for seg in &s[..s.len() - 1] {
                dir.push(seg);
                if let Ok(m) = std::fs::symlink_metadata(&dir)
                    && !m.is_dir()
                {
                    return Err("not-a-directory".into());
                }
            }
            let old = match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => return Err("not-a-file".into()),
                Ok(m) => m.len(),
                Err(_) => 0,
            };
            let used = usage(root);
            if used.saturating_sub(old) + text.len() as u64 > quota {
                return Err("quota".into());
            }
            std::fs::create_dir_all(&dir).map_err(io_error)?;
            // Write to a temporary file, then rename: a crash never leaves
            // half a file.
            static NEXT_TMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT_TMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp = dir.join(format!(".plinth-tmp-{}-{n}", std::process::id()));
            std::fs::write(&tmp, text.as_bytes()).map_err(io_error)?;
            std::fs::rename(&tmp, &p).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                io_error(e)
            })?;
            Ok(String::new())
        }
        Op::List(dir) => {
            let s = checked(&dir, true)?;
            let p = resolve(root, &s);
            if s.is_empty() && !p.exists() {
                return Ok("[]".into());
            }
            let meta = std::fs::symlink_metadata(&p).map_err(io_error)?;
            if !meta.is_dir() {
                return Err("not-a-directory".into());
            }
            let mut out = Vec::new();
            for e in std::fs::read_dir(&p).map_err(io_error)? {
                let e = e.map_err(io_error)?;
                let Ok(name) = e.file_name().into_string() else { continue };
                // Names that the app could not have made (a temporary file,
                // something another program put here) are not listed.
                if check_segment(&name).is_err() || name.starts_with(".plinth-tmp-") {
                    continue;
                }
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() || m.is_file() {
                    out.push((name.clone(), entry_json(&name, &m)));
                }
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(serde_json::Value::Array(out.into_iter().map(|(_, v)| v).collect()).to_string())
        }
        Op::Stat(path) => {
            let s = checked(&path, false)?;
            let p = resolve(root, &s);
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() || m.is_file() => Ok(entry_json(s.last().expect("not empty"), &m).to_string()),
                Ok(_) => Ok("null".into()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok("null".into()),
                Err(e) => Err(io_error(e)),
            }
        }
        Op::Remove(path) => {
            let s = checked(&path, false)?;
            let p = resolve(root, &s);
            match std::fs::symlink_metadata(&p) {
                Ok(m) if m.is_dir() => std::fs::remove_dir_all(&p).map_err(io_error)?,
                Ok(_) => std::fs::remove_file(&p).map_err(io_error)?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
                Err(e) => return Err(io_error(e)),
            }
            // Remove the directories that are now empty, up to the root.
            let mut dir = p.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if d == root || !d.starts_with(root) {
                    break;
                }
                if std::fs::remove_dir(&d).is_err() {
                    break; // not empty
                }
                dir = d.parent().map(Path::to_path_buf);
            }
            Ok(String::new())
        }
    }
}

/// The bytes that the files of the space use. A walk of the folder: a
/// private space is small (the quota is 50 MiB), and the worker thread
/// does it, not the UI thread.
fn usage(root: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plinth-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn ok_text(v: Value) -> String {
        match v {
            Value::List(items) => match (&items[0], &items[2], &items[3]) {
                (Value::Bool(true), Value::Str(t), Value::Null) => t.clone(),
                _ => panic!("expected success, got {items:?}"),
            },
            v => panic!("not a result list: {v:?}"),
        }
    }

    fn err_text(v: Value) -> String {
        match v {
            Value::List(items) => match (&items[0], &items[3]) {
                (Value::Bool(false), Value::Str(e)) => e.clone(),
                _ => panic!("expected an error, got {items:?}"),
            },
            v => panic!("not a result list: {v:?}"),
        }
    }

    #[test]
    fn path_rules() {
        let bad = [
            ("", "empty"),
            ("/etc/passwd", "absolute"),
            ("a\\b", "backslash"),
            ("C:/x", "reserved character"),
            ("c:", "reserved character"),
            ("a/../b", "dot segment"),
            ("..", "dot segment"),
            ("./a", "dot segment"),
            ("a//b", "empty segment"),
            ("a/", "empty segment"),
            ("a\0b", "control character"),
            ("a\nb", "control character"),
            ("a\u{7f}", "control character"),
            ("a\u{85}", "control character"),
            ("file:stream", "reserved character"),
            ("a*", "reserved character"),
            ("a?", "reserved character"),
            ("a<b>", "reserved character"),
            ("a|b", "reserved character"),
            ("\"q\"", "reserved character"),
            ("name.", "trailing dot or space"),
            ("name ", "trailing dot or space"),
            ("CON", "device name"),
            ("nul.txt", "device name"),
            ("com1", "device name"),
            ("LPT9.md", "device name"),
        ];
        for (p, why) in bad {
            assert_eq!(check_path(p, false), Err(why), "{p:?}");
        }
        assert_eq!(check_path(&"a/".repeat(40).trim_end_matches('/').to_owned(), false), Err("too deep"));
        assert_eq!(check_path(&"x".repeat(256), false), Err("segment too long"));
        assert_eq!(check_path(&"abcd/".repeat(300), false), Err("too long"));
        assert_eq!(check_path("", true), Ok(vec![]));
        assert_eq!(check_path("notes/today.md", false), Ok(vec!["notes", "today.md"]));
        assert_eq!(check_path(".hidden", false), Ok(vec![".hidden"]));
        assert_eq!(check_path("...x", false), Ok(vec!["...x"]));
        assert_eq!(check_path("console.md", false), Ok(vec!["console.md"]));
        assert_eq!(check_path("com10", false), Ok(vec!["com10"]));
        assert_eq!(check_path("caf\u{e9}/na\u{ef}ve", false), Ok(vec!["caf\u{e9}", "na\u{ef}ve"]));
        // Not decoded: `%2e%2e` is a plain name.
        assert_eq!(check_path("%2e%2e", false), Ok(vec!["%2e%2e"]));
    }

    #[test]
    fn owners_and_apps_get_different_folders() {
        let d = Path::new("data");
        let a = private_space_dir(d, &Owner::Publisher("ed25519:AAA".into()), "com.example.notes");
        let b = private_space_dir(d, &Owner::Publisher("ed25519:BBB".into()), "com.example.notes");
        let c = private_space_dir(d, &Owner::Publisher("ed25519:AAA".into()), "com.example.other");
        let u = private_space_dir(d, &Owner::of_package(None, b"package bytes"), "com.example.notes");
        let dev = private_space_dir(d, &Owner::Dev, "com.example.notes");
        let all = [&a, &b, &c, &u, &dev];
        for (i, x) in all.iter().enumerate() {
            for y in &all[i + 1..] {
                assert_ne!(x, y);
            }
        }
        assert!(a.starts_with(d.join("spaces").join("private")));
        assert_eq!(private_space_dir(d, &Owner::Dev, "Com.Example.Notes"), dev, "the id is lowercased");
        let odd = private_space_dir(d, &Owner::Dev, "../evil");
        assert_eq!(odd.parent(), Some(d.join("spaces").join("private").join("dev").as_path()));
        assert!(odd.file_name().unwrap().to_string_lossy().starts_with("id-"));
        assert_eq!(Owner::of_package(Some("ed25519:K"), b"x"), Owner::Publisher("ed25519:K".into()));
    }

    #[test]
    fn read_write_list_stat_remove() {
        let root = temp("ops");
        let f = Files::open(root.clone(), DEFAULT_QUOTA);
        assert_eq!(ok_text(f.run_now(Op::List(String::new()))), "[]");
        assert_eq!(err_text(f.run_now(Op::Read("a.md".into()))), "not-found");
        assert_eq!(ok_text(f.run_now(Op::Write("notes/a.md".into(), "hello é".into()))), "");
        assert_eq!(ok_text(f.run_now(Op::Read("notes/a.md".into()))), "hello é");
        assert_eq!(ok_text(f.run_now(Op::List(String::new()))), r#"[{"kind":"dir","name":"notes","size":0}]"#);
        assert_eq!(ok_text(f.run_now(Op::List("notes".into()))), r#"[{"kind":"file","name":"a.md","size":8}]"#);
        assert_eq!(ok_text(f.run_now(Op::Stat("notes/a.md".into()))), r#"{"kind":"file","name":"a.md","size":8}"#);
        assert_eq!(ok_text(f.run_now(Op::Stat("nope".into()))), "null");
        assert_eq!(err_text(f.run_now(Op::Read("notes".into()))), "not-a-file");
        assert_eq!(err_text(f.run_now(Op::List("notes/a.md".into()))), "not-a-directory");
        assert_eq!(err_text(f.run_now(Op::Write("notes/a.md/x".into(), "".into()))), "not-a-directory");
        assert_eq!(err_text(f.run_now(Op::Write("notes".into(), "".into()))), "not-a-file");
        assert_eq!(err_text(f.run_now(Op::List("missing".into()))), "not-found");
        assert_eq!(err_text(f.run_now(Op::Read("../x".into()))), "invalid-path: dot segment");
        assert_eq!(ok_text(f.run_now(Op::Remove("notes/a.md".into()))), "");
        // The empty directory went with its last file.
        assert_eq!(ok_text(f.run_now(Op::List(String::new()))), "[]");
        assert_eq!(ok_text(f.run_now(Op::Remove("notes/a.md".into()))), "");
        ok_text(f.run_now(Op::Write("d/e/f.txt".into(), "1".into())));
        ok_text(f.run_now(Op::Remove("d".into())));
        assert_eq!(ok_text(f.run_now(Op::List(String::new()))), "[]");
        // Not UTF-8 (put there by another program).
        std::fs::write(root.join("bin"), [0xff, 0xfe]).unwrap();
        assert_eq!(err_text(f.run_now(Op::Read("bin".into()))), "not-text");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn quota_and_size_limit() {
        let root = temp("quota");
        let f = Files::open(root.clone(), 10);
        ok_text(f.run_now(Op::Write("a".into(), "12345".into())));
        ok_text(f.run_now(Op::Write("b".into(), "12345".into())));
        assert_eq!(err_text(f.run_now(Op::Write("c".into(), "1".into()))), "quota");
        // Replacing a file counts only the difference.
        ok_text(f.run_now(Op::Write("a".into(), "54321".into())));
        ok_text(f.run_now(Op::Remove("b".into())));
        ok_text(f.run_now(Op::Write("c".into(), "1".into())));
        let big = Files::open(temp("big"), u64::MAX);
        assert_eq!(err_text(big.run_now(Op::Write("x".into(), "a".repeat(MAX_FILE as usize + 1)))), "too-large");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn denied_calls_answer_with_the_reason() {
        let results: Results = Arc::new(Mutex::new(Vec::new()));
        let f = Files::open(temp("denied"), DEFAULT_QUOTA);
        f.submit(&Policy::new(["store.kv"]), 1, Op::Read("a".into()), &results);
        let mut refused = Policy::new([FILES_PRIVATE]);
        refused.refuse(FILES_PRIVATE);
        f.submit(&refused, 2, Op::Read("a".into()), &results);
        Files::unavailable().submit(&Policy::new([FILES_PRIVATE]), 3, Op::Read("a".into()), &results);
        let r = results.lock().unwrap();
        assert_eq!(err_text(r[0].1.clone()), "denied:undeclared");
        assert_eq!(err_text(r[1].1.clone()), "denied:refused");
        assert_eq!(err_text(r[2].1.clone()), "denied:unsupported");
    }

    #[test]
    fn the_worker_keeps_the_order_of_calls() {
        let root = temp("order");
        let results: Results = Arc::new(Mutex::new(Vec::new()));
        let f = Files::open(root.clone(), DEFAULT_QUOTA);
        let policy = Policy::new([FILES_PRIVATE]);
        for i in 0..20u32 {
            f.submit(&policy, i * 2 + 1, Op::Write("n".into(), i.to_string()), &results);
            f.submit(&policy, i * 2 + 2, Op::Read("n".into()), &results);
        }
        let start = std::time::Instant::now();
        while results.lock().unwrap().len() < 40 {
            assert!(start.elapsed().as_secs() < 10, "the worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let r = results.lock().unwrap();
        for i in 0..20u32 {
            let (id, v) = &r[(i * 2 + 1) as usize];
            assert_eq!(*id, i * 2 + 2);
            assert_eq!(ok_text(v.clone()), i.to_string());
        }
        drop(r);
        let _ = std::fs::remove_dir_all(&root);
    }
}
