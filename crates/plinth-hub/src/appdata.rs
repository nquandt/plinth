//! The data that an installed app owns on the desktop (`docs/STORAGE.md`
//! S13): where it is, how big it is, how to export it, how to delete it, and
//! how to tell that the app is running.
//!
//! The layout comes from the code of the host, not from a guess:
//!
//! | What | Where (under the data directory, `plinth_runner_wasmtime::kv::data_dir()`) |
//! |---|---|
//! | `store.kv` | `apps/<app id>/kv.json`, and `apps/<app id>/data-version` |
//! | Private files | `spaces/private/<owner>/<app id>/` (`files::private_space_dir`) |
//! | `store.sql` | `spaces/private/<owner>/<app id>+sql/store.sqlite` (`sql::database_dir`) |
//!
//! `<owner>` is the publisher key of a signed package, or the digest of an
//! unsigned package. So one app can have more than one space. The Hub library
//! records the signer and the digest of each installed version; that is how
//! `Hub::owners` finds them. The host keeps no window state on disk, and the
//! secrets are in the system keychain, not here.
//!
//! The functions take the data directory as an argument. A test uses a
//! temporary directory. Nothing here reads a path from an app: the only
//! inputs are the app id (checked by `check_app_id`) and the owners from the
//! library.

use anyhow::{Context as _, Result, bail};
use plinth_runner_wasmtime::files::{Owner, check_path, private_space_dir};
use plinth_runner_wasmtime::sql;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The default quota of the data of one app (each store: the private files,
/// `store.kv` and `store.sql`), in bytes. `docs/STORAGE.md` S13.
pub const DEFAULT_DATA_QUOTA: u64 = 100 << 20;

/// The format name in `manifest.json` of an export.
pub const EXPORT_FORMAT: &str = "plinth.export/1";

/// Checks an app id before the host builds a path from it. The id must be
/// one plain file name: ASCII letters, digits, `.`, `_`, `-` and `+`; at most
/// 128 bytes; not only dots; no trailing dot; no Windows device name.
pub fn check_app_id(id: &str) -> Result<()> {
    let plain = !id.is_empty()
        && id.len() <= 128
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
        && !id.bytes().all(|b| b == b'.')
        && !id.ends_with('.');
    // The path rules of `plinth:files` also refuse a device name (`CON`).
    if !plain || !matches!(check_path(id, false).map(|s| s.len()), Ok(1)) {
        bail!("`{id}` is not a valid app id");
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One private space of an app: its files folder and its database folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    /// The folder name of the owner (`key-…`, `pkg-…`).
    pub owner: String,
    /// `spaces/private/<owner>/<app id>/`.
    pub files: PathBuf,
    /// `spaces/private/<owner>/<app id>+sql/`.
    pub database: PathBuf,
}

/// All places where one app keeps its data.
#[derive(Debug, Clone)]
pub struct AppDataLayout {
    pub id: String,
    data_dir: PathBuf,
    /// `apps/<app id>/`.
    pub kv: PathBuf,
    pub spaces: Vec<Space>,
}

impl AppDataLayout {
    /// The layout of `id` under `data_dir`, with one space for each of
    /// `owners`. Fails for an app id that is not a plain file name.
    pub fn new(data_dir: &Path, id: &str, owners: &[Owner]) -> Result<Self> {
        check_app_id(id)?;
        let kv = data_dir.join("apps").join(id);
        let mut spaces: Vec<Space> = Vec::new();
        for owner in owners {
            let files = private_space_dir(data_dir, owner, id);
            let owner_name = files.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if spaces.iter().any(|s| s.files == files) {
                continue;
            }
            let database = sql::database_dir(&files);
            spaces.push(Space { owner: owner_name, files, database });
        }
        let layout = Self { id: id.to_owned(), data_dir: data_dir.to_owned(), kv, spaces };
        for folder in layout.folders() {
            layout.check_inside(folder)?;
        }
        Ok(layout)
    }

    /// Every folder of the app, in the order of deletion: databases, files,
    /// then the kv folder.
    pub fn folders(&self) -> Vec<&Path> {
        let mut out: Vec<&Path> = Vec::new();
        for s in &self.spaces {
            out.push(&s.database);
        }
        for s in &self.spaces {
            out.push(&s.files);
        }
        out.push(&self.kv);
        out
    }

    /// A folder must be below the data directory, with no `..`, and must not
    /// be the data directory itself or one of its two top folders.
    fn check_inside(&self, folder: &Path) -> Result<()> {
        let rel = folder.strip_prefix(&self.data_dir).map_err(|_| anyhow::anyhow!("{} is outside the data directory", folder.display()))?;
        let parts: Vec<_> = rel.components().collect();
        let normal = parts.iter().all(|c| matches!(c, std::path::Component::Normal(_)));
        let top = parts.first().map(|c| c.as_os_str().to_string_lossy().into_owned());
        let deep_enough = match top.as_deref() {
            Some("apps") => parts.len() == 2,
            Some("spaces") => parts.len() == 4,
            _ => false,
        };
        if !normal || !deep_enough {
            bail!("{} is not a data folder of an app", folder.display());
        }
        Ok(())
    }

    /// Whether any data folder of the app exists.
    pub fn exists(&self) -> bool {
        self.folders().iter().any(|f| f.exists())
    }

    /// The bytes of all files in the data folders.
    pub fn size(&self) -> u64 {
        self.folders().iter().map(|f| folder_size(f)).sum()
    }

    /// Deletes all data folders. Returns the bytes that it removed. A folder
    /// that does not exist is not an error. A link is removed as a link; the
    /// host never follows it. It tries every folder, then reports the first
    /// failure (for example a file that another process holds open).
    pub fn delete(&self) -> Result<u64> {
        let before = self.size();
        let mut first_error: Option<anyhow::Error> = None;
        for folder in self.folders() {
            self.check_inside(folder)?;
            let result = match std::fs::symlink_metadata(folder) {
                Ok(m) if m.file_type().is_symlink() => std::fs::remove_dir(folder).or_else(|_| std::fs::remove_file(folder)),
                Ok(_) => std::fs::remove_dir_all(folder),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
            if let Err(e) = result
                && first_error.is_none()
            {
                first_error = Some(anyhow::Error::new(e).context(format!("delete {}", folder.display())));
            }
        }
        // Remove the owner folders that are now empty (`remove_dir` fails
        // for a folder that is not empty, which is what we want).
        for s in &self.spaces {
            if let Some(owner_dir) = s.files.parent() {
                let _ = std::fs::remove_dir(owner_dir);
            }
        }
        match first_error {
            Some(e) => Err(e),
            None => Ok(before),
        }
    }

    /// Writes the data of the app to the zip file `out` (`docs/STORAGE.md`
    /// S13): `manifest.json`, `kv.json`, `sql/<owner>/store.sqlite` (a
    /// consistent copy, made with `VACUUM INTO`) and
    /// `files/<owner>/<path>`. The secrets are not in it. `version` is the
    /// version of the app that goes in the manifest.
    pub fn export_zip(&self, version: &str, out: &Path) -> Result<ExportReport> {
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        // Never write the archive into the data that it copies.
        let out_abs = std::path::absolute(out).unwrap_or_else(|_| out.to_owned());
        for folder in self.folders() {
            let folder_abs = std::path::absolute(folder).unwrap_or_else(|_| folder.to_owned());
            if out_abs.starts_with(&folder_abs) {
                bail!("the export file must not be inside the data of the app ({})", folder.display());
            }
        }
        let tmp = out.with_extension("zip.part");
        let result = self.write_zip(version, &tmp);
        match result {
            Ok(report) => {
                std::fs::rename(&tmp, out).with_context(|| format!("rename {} to {}", tmp.display(), out.display()))?;
                Ok(report)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    fn write_zip(&self, version: &str, path: &Path) -> Result<ExportReport> {
        let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        let deflated = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut report = ExportReport::default();
        let mut spaces_json = Vec::new();

        // kv.json: read whole, and check that it parses (the app can write it
        // at the same moment).
        let kv_path = self.kv.join("kv.json");
        match read_json_file(&kv_path) {
            Ok(Some((bytes, entries))) => {
                zip.start_file("kv.json", deflated)?;
                zip.write_all(&bytes)?;
                report.kv_entries = Some(entries);
            }
            Ok(None) => {}
            Err(e) => return Err(e),
        }

        for space in &self.spaces {
            let mut files = 0u64;
            let mut stack = vec![(space.files.clone(), String::new())];
            let mut entries: Vec<(PathBuf, String)> = Vec::new();
            while let Some((dir, prefix)) = stack.pop() {
                let Ok(read) = std::fs::read_dir(&dir) else { continue };
                for e in read.flatten() {
                    let Ok(name) = e.file_name().into_string() else { continue };
                    let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
                    let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
                    if meta.is_dir() {
                        stack.push((e.path(), rel));
                    } else if meta.is_file() && !name.starts_with(".plinth-tmp-") {
                        entries.push((e.path(), rel));
                    }
                }
            }
            entries.sort_by(|a, b| a.1.cmp(&b.1));
            for (path, rel) in entries {
                zip.start_file(format!("files/{}/{rel}", space.owner), deflated)?;
                let mut f = File::open(&path).with_context(|| format!("read {}", path.display()))?;
                report.bytes += std::io::copy(&mut f, &mut zip).with_context(|| format!("copy {}", path.display()))?;
                files += 1;
            }
            report.files += files;

            let db = space.database.join(sql::FILE_NAME);
            let mut has_db = false;
            if db.is_file() {
                let copy = copy_database(&db)?;
                zip.start_file(format!("sql/{}/{}", space.owner, sql::FILE_NAME), deflated)?;
                let mut f = File::open(&copy).with_context(|| format!("read {}", copy.display()))?;
                let n = std::io::copy(&mut f, &mut zip).context("copy the database")?;
                drop(f);
                let _ = std::fs::remove_file(&copy);
                report.bytes += n;
                report.databases += 1;
                has_db = true;
            }
            spaces_json.push(serde_json::json!({ "owner": space.owner, "files": files, "database": has_db }));
        }

        let now = crate::now();
        let manifest = serde_json::json!({
            "format": EXPORT_FORMAT,
            "app": self.id,
            "version": version,
            "exported": iso8601(now),
            "exportedUnix": now,
            "kv": report.kv_entries.map(|n| serde_json::json!({ "entries": n })),
            "spaces": spaces_json,
            "secrets": "not included",
        });
        zip.start_file("manifest.json", deflated)?;
        zip.write_all(serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        zip.finish()?.sync_all().ok();
        Ok(report)
    }
}

/// What an export wrote.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExportReport {
    /// The number of keys in `kv.json`, if the app has a kv file.
    pub kv_entries: Option<u64>,
    /// The number of files from the private spaces.
    pub files: u64,
    /// The number of SQL databases.
    pub databases: u64,
    /// The bytes of files and databases (before compression).
    pub bytes: u64,
}

/// `kv.json` and its number of keys. `None` when the file does not exist.
/// The app writes the file in place, so a read can see half a write; this
/// reads again a few times.
fn read_json_file(path: &Path) -> Result<Option<(Vec<u8>, u64)>> {
    let mut last = None;
    for attempt in 0..5 {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow::Error::new(e).context(format!("read {}", path.display()))),
        };
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(v) => return Ok(Some((bytes, v.as_object().map_or(0, |o| o.len() as u64)))),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(20 * (attempt + 1)));
    }
    Err(anyhow::anyhow!("{} is not valid JSON: {}", path.display(), last.map(|e| e.to_string()).unwrap_or_default()))
}

/// A consistent copy of the SQLite database `db` in a new temporary file
/// (`VACUUM INTO`; the app can be using the database). The caller deletes it.
fn copy_database(db: &Path) -> Result<PathBuf> {
    use rusqlite::{Connection, OpenFlags};
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dest = std::env::temp_dir().join(format!("plinth-export-{}-{n}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&dest);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(db, flags).with_context(|| format!("open the database {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    let target = dest.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!("VACUUM INTO '{target}'")).context("copy the database")?;
    Ok(dest)
}

/// The total size of the files below `dir` (links are not followed).
pub fn folder_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else if m.is_file() {
                total += m.len();
            }
        }
    }
    total
}

/// `2026-10-27T12:00:00Z` for Unix seconds.
fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

// -- Is the app running? -----------------------------------------------------------

/// A lock that a running app holds on a small file in the data directory
/// (`locks/<hash of the app id>.lock`). The host takes a shared lock when it
/// starts the app and keeps it until the app stops (or the process ends: the
/// operating system then frees the lock, so a crash cannot leave a stale
/// one). `uninstall` takes an exclusive lock, which fails while any app
/// window holds a shared one.
#[derive(Debug)]
pub struct RunLock {
    _file: File,
}

fn lock_file(data_dir: &Path, id: &str) -> Result<File> {
    check_app_id(id)?;
    let dir = data_dir.join("locks");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(format!("{}.lock", &hex(&Sha256::digest(id.as_bytes()))[..32]));
    std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path).with_context(|| format!("open {}", path.display()))
}

impl RunLock {
    /// A shared lock: "the app runs". Several windows of one app can hold it.
    pub fn running(data_dir: &Path, id: &str) -> Result<RunLock> {
        let file = lock_file(data_dir, id)?;
        match file.try_lock_shared() {
            Ok(()) => Ok(RunLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => bail!("{id} is being uninstalled"),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }

    /// An exclusive lock: nobody runs the app, and nobody can start it while
    /// this value lives. Fails with a clear message if the app runs.
    pub fn exclusive(data_dir: &Path, id: &str) -> Result<RunLock> {
        let file = lock_file(data_dir, id)?;
        match file.try_lock() {
            Ok(()) => Ok(RunLock { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => {
                bail!("{id} is running; close all its windows first, then try again")
            }
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("plinth-appdata-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_names(archive: &zip::ZipArchive<File>) -> Vec<String> {
        archive.file_names().map(|n| n.unwrap().into_owned()).collect()
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn sample(data: &Path, id: &str, owner: &Owner) -> AppDataLayout {
        let layout = AppDataLayout::new(data, id, std::slice::from_ref(owner)).unwrap();
        write(&layout.kv.join("kv.json"), r#"{"a":"1","b":"2"}"#);
        write(&layout.kv.join("data-version"), "1.0.0");
        let space = &layout.spaces[0];
        write(&space.files.join("notes/today.md"), "hello");
        write(&space.files.join("top.txt"), "top");
        let db = sql::Db::open(&space.database, sql::Limits::default()).unwrap();
        db.exec("CREATE TABLE t (n INTEGER)", "").unwrap();
        db.exec("INSERT INTO t VALUES (7)", "").unwrap();
        layout
    }

    #[test]
    fn ids_that_are_not_plain_file_names_are_refused() {
        for bad in ["", ".", "..", "...", "../evil", "a/b", "a\\b", "a:b", "C:", "a b", "con", "NUL", "com1.x", "name.", "a\0b", "é", &"a".repeat(129)] {
            assert!(check_app_id(bad).is_err(), "{bad:?}");
            assert!(AppDataLayout::new(Path::new("data"), bad, &[Owner::Dev]).is_err(), "{bad:?}");
        }
        for good in ["com.example.notes", "dev.plinth-x_y+z", "a", "A.B"] {
            assert!(check_app_id(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn the_layout_is_inside_the_data_directory() {
        let data = Path::new("data");
        let layout = AppDataLayout::new(data, "com.example.notes", &[Owner::Publisher("ed25519:K".into()), Owner::Package("ab".repeat(32))]).unwrap();
        assert_eq!(layout.kv, data.join("apps").join("com.example.notes"));
        assert_eq!(layout.spaces.len(), 2);
        for f in layout.folders() {
            assert!(f.starts_with(data));
        }
        let s = &layout.spaces[0];
        assert_eq!(s.files.parent().unwrap().file_name().unwrap().to_string_lossy(), s.owner);
        assert_eq!(s.database, sql::database_dir(&s.files));
        // The same owner twice is one space.
        let twice = AppDataLayout::new(data, "x", &[Owner::Dev, Owner::Dev]).unwrap();
        assert_eq!(twice.spaces.len(), 1);
    }

    #[test]
    fn delete_removes_only_this_apps_folders() {
        let data = temp("delete");
        let owner = Owner::Publisher("ed25519:KEY".into());
        let mine = sample(&data, "com.example.mine", &owner);
        let other = sample(&data, "com.example.other", &owner);
        // Same id, other publisher: a different space.
        let stranger = sample(&data, "com.example.mine", &Owner::Publisher("ed25519:OTHER".into()));
        // A file next to the data that must survive.
        write(&data.join("keep.txt"), "keep");
        assert!(mine.size() > 0);
        let freed = mine.delete().unwrap();
        assert!(freed > 0);
        assert!(!mine.exists());
        assert!(other.exists(), "another app keeps its data");
        assert!(stranger.exists(), "the same id from another publisher keeps its data");
        assert!(data.join("keep.txt").exists());
        // Deleting twice is fine.
        mine.delete().unwrap();
        other.delete().unwrap();
        stranger.delete().unwrap();
        assert!(!data.join("spaces/private").join(&mine.spaces[0].owner).exists(), "empty owner folders go too");
        assert!(data.join("keep.txt").exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn a_folder_outside_the_layout_is_never_deleted() {
        let data = temp("outside");
        let victim = temp("victim");
        write(&victim.join("precious.txt"), "do not delete");
        let mut layout = AppDataLayout::new(&data, "com.example.a", &[Owner::Dev]).unwrap();
        layout.kv = victim.clone();
        assert!(layout.delete().is_err());
        layout.kv = data.join("apps").join("x").join("y");
        assert!(layout.delete().is_err(), "too deep");
        layout.kv = data.clone();
        assert!(layout.delete().is_err(), "the data directory itself");
        layout.kv = data.join("apps").join("..").join("..");
        assert!(layout.delete().is_err(), "dot-dot");
        assert!(victim.join("precious.txt").exists());
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(&victim);
    }

    #[test]
    fn export_writes_a_zip_with_the_data_and_no_secrets() {
        let data = temp("export");
        let layout = sample(&data, "com.example.notes", &Owner::Publisher("ed25519:KEY".into()));
        let out = temp("export-out").join("sub").join("notes.zip");
        let report = layout.export_zip("1.2.3", &out).unwrap();
        assert_eq!(report.kv_entries, Some(2));
        assert_eq!(report.files, 2);
        assert_eq!(report.databases, 1);
        assert!(!out.with_extension("zip.part").exists());

        let mut archive = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let names = zip_names(&archive);
        let owner = &layout.spaces[0].owner;
        for want in ["manifest.json", "kv.json", &format!("files/{owner}/notes/today.md"), &format!("files/{owner}/top.txt"), &format!("sql/{owner}/store.sqlite")] {
            assert!(names.iter().any(|n| n == want), "{want} missing from {names:?}");
        }
        let read = |archive: &mut zip::ZipArchive<File>, name: &str| {
            let mut s = Vec::new();
            std::io::Read::read_to_end(&mut archive.by_name(name).unwrap(), &mut s).unwrap();
            s
        };
        let manifest: serde_json::Value = serde_json::from_slice(&read(&mut archive, "manifest.json")).unwrap();
        assert_eq!(manifest["format"], EXPORT_FORMAT);
        assert_eq!(manifest["app"], "com.example.notes");
        assert_eq!(manifest["version"], "1.2.3");
        assert_eq!(manifest["secrets"], "not included");
        assert!(manifest["exported"].as_str().unwrap().ends_with('Z'));
        assert_eq!(read(&mut archive, &format!("files/{owner}/notes/today.md")), b"hello");
        assert_eq!(read(&mut archive, "kv.json"), br#"{"a":"1","b":"2"}"#);

        // The database copy opens and holds the row.
        let db_bytes = read(&mut archive, &format!("sql/{owner}/store.sqlite"));
        let copy = data.join("copy.sqlite");
        std::fs::write(&copy, db_bytes).unwrap();
        let conn = rusqlite::Connection::open(&copy).unwrap();
        let n: i64 = conn.query_row("SELECT n FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 7);
        drop(conn);

        // Not inside the data of the app.
        let inside = layout.kv.join("export.zip");
        assert!(layout.export_zip("1", &inside).is_err());
        let _ = std::fs::remove_dir_all(&data);
        let _ = std::fs::remove_dir_all(out.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn export_of_an_app_with_no_data_has_only_a_manifest() {
        let data = temp("export-empty");
        let layout = AppDataLayout::new(&data, "com.example.none", &[Owner::Dev]).unwrap();
        let out = data.join("none.zip");
        let report = layout.export_zip("0.0.1", &out).unwrap();
        assert_eq!(report, ExportReport::default());
        let archive = zip::ZipArchive::new(File::open(&out).unwrap()).unwrap();
        assert_eq!(zip_names(&archive), vec!["manifest.json"]);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn the_run_lock_tells_a_running_app() {
        let data = temp("lock");
        assert!(RunLock::exclusive(&data, "com.example.a").is_ok(), "nobody runs it");
        let one = RunLock::running(&data, "com.example.a").unwrap();
        let two = RunLock::running(&data, "com.example.a").unwrap();
        let err = RunLock::exclusive(&data, "com.example.a").unwrap_err().to_string();
        assert!(err.contains("is running"), "{err}");
        // Another app is not affected.
        assert!(RunLock::exclusive(&data, "com.example.b").is_ok());
        drop(one);
        assert!(RunLock::exclusive(&data, "com.example.a").is_err(), "one window still runs it");
        drop(two);
        let held = RunLock::exclusive(&data, "com.example.a").unwrap();
        assert!(RunLock::running(&data, "com.example.a").unwrap_err().to_string().contains("being uninstalled"));
        drop(held);
        assert!(RunLock::running(&data, "../x").is_err());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn iso_dates() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso8601(951_782_400), "2000-02-29T00:00:00Z");
    }
}
