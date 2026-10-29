//! The Plinth Hub local library (`docs/HUB.md` §4.3, §9, phase H0).
//!
//! This crate is the native, non-UI part of the Hub: the package cache,
//! the library store, the grants store and the blocks store. It has no
//! dependency on gpui or wasmtime; the desktop host and the `plinth hub`
//! CLI build the consent screen and the run path on top of it.
//!
//! All stores live under the Hub data directory (`hub_dir()`): a content-
//! addressed package cache (`packages/<sha256>.plnt`), `library.json`,
//! `grants.json` and `blocks.json`. Every write is atomic (write a temp
//! file, then rename it over the target), so a crash mid-write cannot
//! corrupt a store.

pub mod appdata;
pub mod os;

pub use appdata::{AppDataLayout, DEFAULT_DATA_QUOTA, ExportReport, RunLock};
pub use plinth_runner_wasmtime::files::Owner;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The Hub data directory: `PLINTH_HUB_DIR`, or `%LOCALAPPDATA%\plinth\hub`
/// on Windows and `$XDG_DATA_HOME/plinth/hub` (or `~/.local/share/plinth/hub`)
/// elsewhere. Modeled on `plinth_link::cores::cores_dir`.
pub fn hub_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PLINTH_HUB_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("plinth").join("hub")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` to `path` atomically: a temp file in the same directory,
/// then a rename. A rename within one directory is atomic on every
/// platform this project targets.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let tmp = parent.join(format!(".{}.tmp-{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("rename {} to {}", tmp.display(), path.display()))?;
    Ok(())
}

fn read_json<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    write_atomic(path, text.as_bytes())
}

// -- Library -----------------------------------------------------------------

/// One installed version of an app in the library.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VersionEntry {
    pub version: String,
    /// The SHA-256 of the `.plnt`, hex-encoded; the key into `packages/`.
    pub digest: String,
    /// The capabilities this version's manifest declared, recorded at
    /// install time so a later version's new capabilities can be found
    /// without re-reading the package (`docs/HUB.md` §7.3 step 3).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The signer's key id (`docs/HUB.md` §6.1, phase H1), if the package
    /// was signed. `None` for an unsigned package (still allowed: draft
    /// registries and packages without signatures keep working).
    #[serde(default)]
    pub signer: Option<String>,
}

/// Where the Hub got an app. Only `"file"` exists until H2 (sources).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    File,
}

/// One app in the library (`docs/HUB.md` §9.1, §4.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LibraryEntry {
    pub id: String,
    pub name: String,
    pub versions: Vec<VersionEntry>,
    /// A pinned version, or `None` to always run the latest added version.
    pub pinned: Option<String>,
    pub groups: Vec<String>,
    /// Unix seconds.
    pub added: u64,
    pub source: Source,
    /// The registry (`docs/REGISTRY.md` §9) this app was installed from, if
    /// any; `None` for an app added from a bare file (`plinth hub add`).
    #[serde(default)]
    pub registry: Option<RegistrySource>,
}

/// The registry source of a library app: the configured source name
/// (`docs/REGISTRY.md` §9, `hub source add`) and its base, recorded so
/// `plinth hub update` knows where to look for a newer version.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistrySource {
    pub name: String,
    pub base: String,
}

/// A newer version of a library app that a source lists (`docs/HUB.md`
/// §9.2), as the last update check found it. Stored in `updates.json`, so
/// the Hub UI can show "Update available" without a new check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AvailableUpdate {
    pub version: String,
    /// The name of the source that lists it.
    pub source: String,
    /// The capabilities of the new version that the newest installed
    /// version does not declare (`docs/HUB.md` §7.3 step 3).
    #[serde(default)]
    pub new_capabilities: Vec<String>,
}

/// `MAJOR.MINOR.PATCH` as a comparable tuple; a part that does not parse
/// counts as 0 (the same rule as `plinth_registry::is_newer`).
fn semver_key(v: &str) -> (u64, u64, u64) {
    let mut it = v.split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// True if version `a` is newer than version `b` (semver order).
pub fn is_newer(a: &str, b: &str) -> bool {
    semver_key(a) > semver_key(b)
}

/// The registry sources known to this Hub (`hub source add/list/remove`),
/// stored in `sources.json`: name -> base URL/path, with optional request
/// headers (`docs/HUB.md` §5.4).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Sources(pub BTreeMap<String, SourceRecord>);

/// One entry of `sources.json`. A source with no headers is a plain string,
/// so an older Hub can read the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum SourceRecord {
    Plain(String),
    Full {
        base: String,
        #[serde(default)]
        headers: Vec<HeaderSpec>,
    },
}

impl SourceRecord {
    fn new(base: &str, headers: Vec<HeaderSpec>) -> Self {
        if headers.is_empty() { Self::Plain(base.to_owned()) } else { Self::Full { base: base.to_owned(), headers } }
    }

    pub fn base(&self) -> &str {
        match self {
            Self::Plain(base) | Self::Full { base, .. } => base,
        }
    }

    pub fn headers(&self) -> &[HeaderSpec] {
        match self {
            Self::Plain(_) => &[],
            Self::Full { headers, .. } => headers,
        }
    }
}

/// A request header of a source. It has a `value` (only for a value that is
/// not secret) or a `secret`: the name of an entry in the secret store. The
/// file never holds a secret value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeaderSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// The app id of the host's own secrets (`docs/SECRETS.md`). No app can
/// have this id.
pub const HOST_SECRETS_APP: &str = "plinth.host";

/// The name of the secret for header `header` of source `source`:
/// `source.<source>.<header in lowercase>`. The name must pass the rule for
/// secret names (1 to 64 characters from `a-z A-Z 0-9 . _ -`), so this
/// returns an error for a source name or a header name that does not fit.
pub fn header_secret_name(source: &str, header: &str) -> Result<String> {
    let name = format!("source.{source}.{}", header.to_ascii_lowercase());
    let fits = name.chars().count() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !fits {
        bail!("the source name `{source}` or the header name `{header}` cannot make a secret name (use letters, digits, `.`, `_` and `-`, at most 64 characters in all)");
    }
    Ok(name)
}

/// A new secret store for each call (the system keychain on a desktop).
pub type SecretsFactory = std::sync::Arc<dyn Fn() -> Box<dyn plinth_runner_wasmtime::secrets::SecretsBackend> + Send + Sync>;

impl LibraryEntry {
    /// The version the Hub should run: the pinned one, or the latest added
    /// (the last entry in `versions`, since `add_package` appends).
    pub fn active_version(&self) -> Option<&VersionEntry> {
        if let Some(pinned) = &self.pinned {
            if let Some(v) = self.versions.iter().find(|v| &v.version == pinned) {
                return Some(v);
            }
        }
        self.versions.last()
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Library {
    apps: BTreeMap<String, LibraryEntry>,
    groups: Vec<String>,
}

// -- Grants -------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allowed,
    Refused,
}

/// The user's decision for one (app id, capability) pair
/// (`docs/HUB.md` §4.3, §7.3).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Grant {
    pub decision: Decision,
    /// Unix seconds.
    pub time: u64,
    /// The app version the user saw when they decided.
    pub version_seen: String,
    /// `true` when this decision was made automatically because the
    /// capability's risk is `None` or `Low` (`docs/HUB.md` §7.2: "allowed,
    /// by default"), `false` when the user decided it (the install
    /// consent screen, the first-use prompt, or `plinth hub grants`).
    #[serde(default)]
    pub by_default: bool,
}

/// A capability's full consent status, for the consent screen and
/// `plinth hub grants <id>` (`docs/HUB.md` §7.2): its risk level, and
/// whether and how it was decided.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityStatus {
    pub name: String,
    pub risk: plinth_link::capabilities::Risk,
    /// `None` until the user decides a Medium/High capability (a Low/None
    /// one is decided automatically before this is ever `None` for long:
    /// `add_package` grants it immediately).
    pub decision: Option<Decision>,
    pub by_default: bool,
}

/// The risk level of `capability`, or `Medium` (ask) for a capability this
/// build does not know about: safer to ask than to assume it is harmless.
fn risk_of(capability: &str) -> plinth_link::capabilities::Risk {
    plinth_link::capabilities::info(capability).map(|i| i.risk).unwrap_or(plinth_link::capabilities::Risk::Medium)
}

/// Whether `capability`'s risk means it is granted at install without a
/// question (`docs/HUB.md` §7.2).
pub fn is_low_risk(capability: &str) -> bool {
    matches!(risk_of(capability), plinth_link::capabilities::Risk::None | plinth_link::capabilities::Risk::Low)
}

/// The capabilities in `new` that `old` does not declare, in the order of
/// `new` (`docs/HUB.md` §7.3 step 3). A capability that `new` removes is
/// not in the result: a removal needs no consent.
pub fn added_capabilities(old: &[String], new: &[String]) -> Vec<String> {
    new.iter().filter(|c| !old.contains(c)).cloned().collect()
}

/// The capabilities in `declared` that need the user's decision, in the
/// order of `declared` (`docs/HUB.md` §7.2, §7.3): a Medium or High
/// capability for which `decided` is false. A `None`/`Low` capability is
/// granted by default and never needs a decision. The Hub
/// (`Hub::needs_consent`) and the desktop self-updater
/// (`plinth-host-desktop::updater`) both use this rule.
pub fn undecided(declared: &[String], decided: impl Fn(&str) -> bool) -> Vec<String> {
    declared.iter().filter(|c| !decided(c.as_str()) && !is_low_risk(c)).cloned().collect()
}

/// Whether a declared capability is granted, from its recorded decision
/// (`docs/HUB.md` §7.1, §7.2): allowed is granted, refused is not, and no
/// decision is granted only for a `None`/`Low` capability. The global
/// deny list (`Hub::policy_denied`) is a separate, later step.
pub fn is_granted(capability: &str, decision: Option<Decision>) -> bool {
    match decision {
        Some(Decision::Allowed) => true,
        Some(Decision::Refused) => false,
        None => is_low_risk(capability),
    }
}

/// Key: `"<app id>\u{1f}<capability>"` (the unit separator cannot appear in
/// either an app id or a capability name).
#[derive(Debug, Default, Serialize, Deserialize)]
struct Grants(BTreeMap<String, Grant>);

fn grant_key(id: &str, capability: &str) -> String {
    format!("{id}\u{1f}{capability}")
}

// -- Blocks -------------------------------------------------------------------

/// Blocked app ids and publisher ids (`docs/HUB.md` §4.3, §7.4). Publisher
/// ids stay empty until H1 (signing) gives every package a publisher key.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Blocks {
    pub apps: Vec<String>,
    pub publishers: Vec<String>,
}

// -- The Hub ------------------------------------------------------------------

/// A handle to one Hub data directory. Tests use a temp directory
/// directly; the CLI and the desktop host use `hub_dir()`. Cheap to
/// clone: it is just the directory path, re-read from disk on every call
/// (`HubService` keeps one by value, `docs/HUB.md` §4.1, §12.2).
#[derive(Clone)]
pub struct Hub {
    dir: PathBuf,
}

impl Hub {
    /// Opens (creating if needed) the Hub rooted at `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        Ok(Self { dir })
    }

    /// Opens the default Hub directory (`hub_dir()`).
    pub fn open_default() -> Result<Self> {
        Self::open(hub_dir())
    }

    /// The data directory of this Hub.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn packages_dir(&self) -> PathBuf {
        self.dir.join("packages")
    }
    fn library_path(&self) -> PathBuf {
        self.dir.join("library.json")
    }
    fn grants_path(&self) -> PathBuf {
        self.dir.join("grants.json")
    }
    fn blocks_path(&self) -> PathBuf {
        self.dir.join("blocks.json")
    }
    pub fn sources_path(&self) -> PathBuf {
        self.dir.join("sources.json")
    }

    fn library(&self) -> Result<Library> {
        read_json(&self.library_path())
    }
    fn save_library(&self, lib: &Library) -> Result<()> {
        write_json(&self.library_path(), lib)
    }

    /// Validates `bytes` as a `.plnt` package and links it against an
    /// installed core, to catch a package that cannot run before it is
    /// stored. Adds it to the library (a new version if the app id is
    /// already there) and caches the bytes content-addressed. Returns the
    /// app id.
    pub fn add_package(&self, bytes: &[u8]) -> Result<String> {
        let pkg = plinth_package::Package::read(bytes).map_err(|e| plinth_package::read_error(e, "not a valid .plnt package"))?;
        plinth_link::cores::link_app_with_policy(&pkg.component, &pkg.manifest.core, pkg.manifest.core_digest.as_deref(), pkg.manifest.core_built.as_deref())
            .context("the package does not link against an installed core")?;
        if self.is_blocked(&pkg.manifest.id)? {
            bail!("{} is blocked", pkg.manifest.id);
        }
        // A package with a signature that does not check out (tampered
        // bytes, wrong publisher name, bad digests) is refused outright; an
        // unsigned package (`None`) is still allowed (`docs/HUB.md` §6.1,
        // §6.2 "Unverified publisher").
        let signer = plinth_package::signature::verify(&pkg).context("the package signature does not check out")?;
        if let Some(s) = &signer
            && self.is_publisher_blocked(&s.key)?
        {
            bail!("publisher {} ({}) is blocked", s.publisher, s.key);
        }
        let digest = hex(&Sha256::digest(bytes));
        let cache_path = self.packages_dir().join(format!("{digest}.plnt"));
        if !cache_path.exists() {
            write_atomic(&cache_path, bytes)?;
        }
        let mut lib = self.library()?;
        let entry = lib.apps.entry(pkg.manifest.id.clone()).or_insert_with(|| LibraryEntry {
            id: pkg.manifest.id.clone(),
            name: pkg.manifest.name.clone(),
            versions: Vec::new(),
            pinned: None,
            groups: Vec::new(),
            added: now(),
            source: Source::File,
            registry: None,
        });
        // A new version signed by a different key than the previously
        // installed version is refused (`docs/HUB.md` §6.1: "a later
        // version must have the same key, or a key that the old key has
        // signed"). Key rotation statements are not implemented yet
        // (future work); for now a key change needs a fresh app id.
        if let Some(previous) = entry.versions.last()
            && let (Some(old_key), Some(new)) = (&previous.signer, &signer)
            && old_key != &new.key
        {
            bail!(
                "{id} is signed by a different key ({new_key}) than the installed version ({old_key}); key rotation is not supported yet.                  To install this package, run `plinth hub remove {id}`, then install it again.                  `remove` only takes the app out of the library: the data of the old version stays on disk, but it belongs to the old key, so the new package does not see it.                  To delete that data, run `plinth hub uninstall {id}` instead of `remove` (without `--keep-data`)",
                id = pkg.manifest.id,
                new_key = new.key,
            );
        }
        entry.name = pkg.manifest.name.clone();
        let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
        if !entry.versions.iter().any(|v| v.version == pkg.manifest.version) {
            entry.versions.push(VersionEntry {
                version: pkg.manifest.version.clone(),
                digest,
                capabilities: declared.clone(),
                signer: signer.map(|s| s.key),
            });
        }
        self.save_library(&lib)?;
        self.grant_low_risk_defaults(&pkg.manifest.id, &pkg.manifest.version, &declared)?;
        Ok(pkg.manifest.id)
    }

    /// Grants, "allowed, by default", every declared capability whose risk
    /// is `None` or `Low` and that has no decision yet (`docs/HUB.md`
    /// §7.2). Called from `add_package`, so this covers both a first
    /// install and a new version that adds a Low-risk capability.
    fn grant_low_risk_defaults(&self, id: &str, version: &str, declared: &[String]) -> Result<()> {
        let existing = self.grants(id)?;
        for cap in declared {
            if existing.contains_key(cap.as_str()) {
                continue;
            }
            if is_low_risk(cap) {
                self.set_grant_inner(id, cap, Decision::Allowed, version, true)?;
            }
        }
        Ok(())
    }

    /// The library entries, sorted by app id.
    pub fn list(&self) -> Result<Vec<LibraryEntry>> {
        Ok(self.library()?.apps.into_values().collect())
    }

    pub fn get(&self, id: &str) -> Result<Option<LibraryEntry>> {
        Ok(self.library()?.apps.get(id).cloned())
    }

    /// Removes an app from the library (its cached packages and grants are
    /// left in place; `docs/HUB.md` leaves cache eviction to a later
    /// phase).
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut lib = self.library()?;
        if lib.apps.remove(id).is_none() {
            bail!("{id} is not in the library");
        }
        self.save_library(&lib)?;
        self.set_available_update(id, None)
    }

    // -- Uninstall, export and the data quota (`docs/STORAGE.md` S13) ------

    /// The owners of the private spaces of `id`: one for each installed
    /// version (the signer's key, or the package digest of an unsigned
    /// version). A key change or a new unsigned build gives a new space.
    pub fn owners(&self, id: &str) -> Result<Vec<Owner>> {
        let entry = self.get(id)?.with_context(|| format!("{id} is not in the library"))?;
        let mut owners: Vec<Owner> = Vec::new();
        for v in &entry.versions {
            let owner = match v.signer.as_deref() {
                Some(key) if !key.is_empty() => Owner::Publisher(key.to_owned()),
                _ => Owner::Package(v.digest.clone()),
            };
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        }
        Ok(owners)
    }

    /// The places where `id` keeps its data under `data_dir`.
    pub fn data_layout(&self, id: &str, data_dir: &Path) -> Result<appdata::AppDataLayout> {
        appdata::AppDataLayout::new(data_dir, id, &self.owners(id)?)
    }

    /// Uninstalls `id`: removes the app from the library and, unless
    /// `keep_data`, deletes everything that it owns (`docs/STORAGE.md` S13):
    /// the `store.kv` file, the `store.sql` databases, the private file
    /// spaces, the grants of the app, and (through `remove_secrets`, which
    /// the host gives because the keychain is not in this crate) its
    /// secrets. It always removes the cached packages that no other app
    /// uses.
    ///
    /// The operation first takes an exclusive run lock (`appdata::RunLock`):
    /// if the app runs, it fails and changes nothing. The data goes first
    /// and the library entry last, so after a failure the app is still in
    /// the library and the same command can run again. The block list is not
    /// changed: a block is a safety decision, not app data.
    pub fn uninstall(
        &self,
        id: &str,
        data_dir: &Path,
        keep_data: bool,
        remove_secrets: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<UninstallReport> {
        appdata::check_app_id(id)?;
        if id == HOST_SECRETS_APP {
            bail!("`{id}` is not an app");
        }
        let lib = self.library()?;
        let entry = lib.apps.get(id).with_context(|| format!("{id} is not in the library"))?.clone();
        let layout = self.data_layout(id, data_dir)?;
        let _lock = appdata::RunLock::exclusive(data_dir, id)?;

        let mut report = UninstallReport { data_kept: keep_data, ..UninstallReport::default() };
        if !keep_data {
            report.deleted_bytes = layout.delete()?;
            remove_secrets(id).with_context(|| format!("delete the secrets of {id}"))?;
            report.secrets_removed = true;
            let mut store = self.grants_store()?;
            let prefix = format!("{id}\u{1f}");
            let before = store.0.len();
            store.0.retain(|k, _| !k.starts_with(&prefix));
            report.grants_removed = before - store.0.len();
            if report.grants_removed > 0 {
                write_json(&self.grants_path(), &store)?;
            }
        }

        let mut lib = self.library()?;
        lib.apps.remove(id);
        self.save_library(&lib)?;
        self.set_available_update(id, None)?;
        for v in &entry.versions {
            let used_elsewhere = lib.apps.values().any(|e| e.versions.iter().any(|o| o.digest == v.digest));
            let plain_hex = !v.digest.is_empty() && v.digest.bytes().all(|b| b.is_ascii_hexdigit());
            if !used_elsewhere && plain_hex {
                let path = self.packages_dir().join(format!("{}.plnt", v.digest));
                if std::fs::remove_file(&path).is_ok() {
                    report.packages_removed += 1;
                }
            }
        }
        let _ = std::fs::remove_file(self.dir.join("icons").join(format!("{id}.ico")));
        Ok(report)
    }

    /// Writes the data of `id` to the zip file `out` (`appdata::AppDataLayout::export_zip`).
    /// The secrets are not exported. The manifest names the newest installed version.
    pub fn export_data(&self, id: &str, data_dir: &Path, out: &Path) -> Result<appdata::ExportReport> {
        let layout = self.data_layout(id, data_dir)?;
        let version = self.newest_installed(id)?.map(|v| v.version).unwrap_or_default();
        layout.export_zip(&version, out)
    }

    /// The data quota of one app, in bytes, for all stores together (the private
    /// files, `store.kv` and `store.sql`): `PLINTH_DATA_QUOTA_MIB`, else the
    /// `dataQuotaMib` of `policy.json`, else `DEFAULT_DATA_QUOTA`.
    pub fn data_quota(&self) -> u64 {
        data_quota_in(&self.dir)
    }

    /// Sets (`Some`, in MiB; at least 1) or clears (`None`) the data quota
    /// in `policy.json`.
    pub fn set_data_quota_mib(&self, mib: Option<u64>) -> Result<()> {
        if mib == Some(0) {
            bail!("the data quota must be at least 1 MiB");
        }
        let mut store = self.policy_store()?;
        store.data_quota_mib = mib;
        write_json(&self.policy_path(), &store)
    }

    /// The package bytes for the app's active version (`docs/HUB.md`
    /// §9.1).
    pub fn package(&self, id: &str) -> Result<Vec<u8>> {
        let version = self.runnable_version(id)?;
        let path = self.packages_dir().join(format!("{}.plnt", version.digest));
        std::fs::read(&path).with_context(|| format!("read {}", path.display()))
    }

    /// The newest installed version of `id` by semver (not necessarily the
    /// active one: a pin can hold an older version).
    pub fn newest_installed(&self, id: &str) -> Result<Option<VersionEntry>> {
        let Some(entry) = self.get(id)? else { return Ok(None) };
        Ok(entry.versions.iter().max_by(|a, b| semver_key(&a.version).cmp(&semver_key(&b.version))).cloned())
    }

    // -- Available updates (`docs/HUB.md` §9.2) --------------------------

    fn updates_path(&self) -> PathBuf {
        self.dir.join("updates.json")
    }

    /// The updates that the last check found, by app id (`updates.json`).
    pub fn available_updates(&self) -> Result<BTreeMap<String, AvailableUpdate>> {
        read_json(&self.updates_path())
    }

    /// The update that the last check found for `id`, if any.
    pub fn available_update(&self, id: &str) -> Result<Option<AvailableUpdate>> {
        Ok(self.available_updates()?.remove(id))
    }

    /// Records (`Some`) or clears (`None`) the available update of `id`.
    pub fn set_available_update(&self, id: &str, update: Option<AvailableUpdate>) -> Result<()> {
        let mut store = self.available_updates()?;
        let changed = match update {
            Some(u) => store.insert(id.to_owned(), u.clone()).as_ref() != Some(&u),
            None => store.remove(id).is_some(),
        };
        if changed { write_json(&self.updates_path(), &store) } else { Ok(()) }
    }

    /// Writes the icon of `id` (the manifest's `icon`, a PNG in the
    /// package) as `icons/<id>.ico` in the Hub directory, for a shortcut
    /// (`docs/HUB.md` §10). `Ok(None)` when the app has no icon, or when its
    /// icon is not a PNG image.
    pub fn write_app_icon(&self, id: &str) -> Result<Option<PathBuf>> {
        let bytes = self.package(id)?;
        let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("read the package for {id}"))?;
        let Some(icon) = &pkg.manifest.icon else { return Ok(None) };
        let icon = icon.trim_start_matches("./");
        let Some((_, png)) = pkg.assets.iter().find(|(path, _)| path == icon) else { return Ok(None) };
        let Some(ico) = os::png_to_ico(png) else { return Ok(None) };
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')) {
            bail!("`{id}` is not a valid app id");
        }
        let path = self.dir.join("icons").join(format!("{id}.ico"));
        write_atomic(&path, &ico)?;
        Ok(Some(path))
    }

    /// Pins `id` to `version`, or unpins it (`version: None` runs latest).
    pub fn pin(&self, id: &str, version: Option<String>) -> Result<()> {
        let mut lib = self.library()?;
        let entry = lib.apps.get_mut(id).with_context(|| format!("{id} is not in the library"))?;
        if let Some(v) = &version {
            if !entry.versions.iter().any(|e| &e.version == v) {
                bail!("{id} has no version {v}");
            }
        }
        entry.pinned = version;
        self.save_library(&lib)
    }

    // -- Grants ---------------------------------------------------------

    fn grants_store(&self) -> Result<Grants> {
        read_json(&self.grants_path())
    }

    /// Every grant decision recorded for `id`, by capability name.
    pub fn grants(&self, id: &str) -> Result<BTreeMap<String, Grant>> {
        let store = self.grants_store()?;
        let prefix = format!("{id}\u{1f}");
        Ok(store.0.into_iter().filter_map(|(k, v)| k.strip_prefix(&prefix).map(|cap| (cap.to_owned(), v))).collect())
    }

    /// The decision for one (app, capability) pair, if the user has made
    /// one.
    pub fn grant(&self, id: &str, capability: &str) -> Result<Option<Grant>> {
        Ok(self.grants_store()?.0.get(&grant_key(id, capability)).cloned())
    }

    /// Records the user's decision for `(id, capability)`. Always `by_default:
    /// false`: a call to this public method is the user deciding, whether
    /// from the consent screen or `plinth hub grants ... allow|refuse`
    /// (which lets the user refuse even a Low-risk capability that was
    /// granted by default, `docs/HUB.md` §7.2).
    pub fn set_grant(&self, id: &str, capability: &str, decision: Decision, version_seen: &str) -> Result<()> {
        self.set_grant_inner(id, capability, decision, version_seen, false)
    }

    fn set_grant_inner(&self, id: &str, capability: &str, decision: Decision, version_seen: &str, by_default: bool) -> Result<()> {
        let mut store = self.grants_store()?;
        store.0.insert(grant_key(id, capability), Grant { decision, time: now(), version_seen: version_seen.to_owned(), by_default });
        write_json(&self.grants_path(), &store)
    }

    /// The full consent status of each of `declared`'s capabilities: risk
    /// level, decision (if any), and whether it was decided by default
    /// (`docs/HUB.md` §7.2). Used by the consent screen and `plinth hub
    /// grants <id>`.
    pub fn capability_report(&self, id: &str, declared: &[String]) -> Result<Vec<CapabilityStatus>> {
        let grants = self.grants(id)?;
        Ok(declared
            .iter()
            .map(|name| {
                let grant = grants.get(name);
                CapabilityStatus {
                    name: name.clone(),
                    risk: risk_of(name),
                    decision: grant.map(|g| g.decision),
                    by_default: grant.map(|g| g.by_default).unwrap_or(false),
                }
            })
            .collect())
    }

    // -- Blocks -----------------------------------------------------------

    fn blocks_store(&self) -> Result<Blocks> {
        read_json(&self.blocks_path())
    }

    /// Whether `id` must not run: the app is blocked, or the key that signed
    /// one of its installed versions is a blocked publisher (`docs/HUB.md`
    /// §7.4: "block a publisher: all its apps and future apps").
    pub fn is_blocked(&self, id: &str) -> Result<bool> {
        if self.is_app_blocked(id)? {
            return Ok(true);
        }
        self.is_publisher_of_blocked(id)
    }

    /// Whether `id` itself is on the block list (not its publisher).
    pub fn is_app_blocked(&self, id: &str) -> Result<bool> {
        Ok(self.blocks_store()?.apps.iter().any(|a| a == id))
    }

    /// Whether a key that signed an installed version of `id` is a blocked
    /// publisher. `false` for an app that is not in the library or that is
    /// unsigned.
    pub fn is_publisher_of_blocked(&self, id: &str) -> Result<bool> {
        let blocks = self.blocks_store()?;
        if blocks.publishers.is_empty() {
            return Ok(false);
        }
        let Some(entry) = self.get(id)? else { return Ok(false) };
        Ok(entry.versions.iter().filter_map(|v| v.signer.as_deref()).any(|key| blocks.publishers.iter().any(|p| p == key)))
    }

    pub fn is_publisher_blocked(&self, publisher: &str) -> Result<bool> {
        Ok(self.blocks_store()?.publishers.iter().any(|p| p == publisher))
    }

    pub fn block_app(&self, id: &str) -> Result<()> {
        let mut blocks = self.blocks_store()?;
        if !blocks.apps.iter().any(|a| a == id) {
            blocks.apps.push(id.to_owned());
        }
        write_json(&self.blocks_path(), &blocks)
    }

    pub fn unblock_app(&self, id: &str) -> Result<()> {
        let mut blocks = self.blocks_store()?;
        blocks.apps.retain(|a| a != id);
        write_json(&self.blocks_path(), &blocks)
    }

    pub fn block_publisher(&self, publisher: &str) -> Result<()> {
        let mut blocks = self.blocks_store()?;
        if !blocks.publishers.iter().any(|p| p == publisher) {
            blocks.publishers.push(publisher.to_owned());
        }
        write_json(&self.blocks_path(), &blocks)
    }

    pub fn unblock_publisher(&self, publisher: &str) -> Result<()> {
        let mut blocks = self.blocks_store()?;
        blocks.publishers.retain(|p| p != publisher);
        write_json(&self.blocks_path(), &blocks)
    }

    // -- Groups -------------------------------------------------------------

    pub fn groups(&self) -> Result<Vec<String>> {
        Ok(self.library()?.groups)
    }

    pub fn create_group(&self, name: &str) -> Result<()> {
        let mut lib = self.library()?;
        if !lib.groups.iter().any(|g| g == name) {
            lib.groups.push(name.to_owned());
        }
        self.save_library(&lib)
    }

    /// Takes `id` out of `group`. The group stays, also when it is empty.
    pub fn remove_from_group(&self, id: &str, group: &str) -> Result<()> {
        let mut lib = self.library()?;
        let entry = lib.apps.get_mut(id).with_context(|| format!("{id} is not in the library"))?;
        entry.groups.retain(|g| g != group);
        self.save_library(&lib)
    }

    pub fn add_to_group(&self, id: &str, group: &str) -> Result<()> {
        let mut lib = self.library()?;
        if !lib.groups.iter().any(|g| g == group) {
            lib.groups.push(group.to_owned());
        }
        let entry = lib.apps.get_mut(id).with_context(|| format!("{id} is not in the library"))?;
        if !entry.groups.iter().any(|g| g == group) {
            entry.groups.push(group.to_owned());
        }
        self.save_library(&lib)
    }

    /// Builds a runtime policy from the manifest's declared capabilities
    /// and the recorded grants (`docs/HUB.md` §7.1, §12.4): declared and
    /// allowed is granted; declared and refused, or declared with no
    /// decision yet, is refused (the caller shows consent first and calls
    /// `set_grant` before this, so "no decision" should not normally
    /// reach here for a library app; `needs_consent` below finds that
    /// case ahead of time).
    /// `None` is granted as declared (no decision needed, `docs/HUB.md`
    /// §7.2), `Low` is granted unless the user explicitly refused it, and
    /// `Medium`/`High` need an explicit grant. A capability on the global
    /// deny list (`policy deny`, `docs/HUB.md` §7.4) is refused regardless
    /// of any grant.
    pub fn policy_for(&self, id: &str, declared: &[String]) -> Result<plinth_runner_wasmtime::policy::Policy> {
        let grants = self.grants(id)?;
        let denied = self.policy_denied()?;
        let mut policy = plinth_runner_wasmtime::policy::Policy::new(declared.iter().cloned());
        for cap in declared {
            if denied.iter().any(|d| d == cap) {
                policy.refuse(cap.clone());
                continue;
            }
            if !is_granted(cap, grants.get(cap).map(|g| g.decision)) {
                policy.refuse(cap.clone());
            }
        }
        Ok(policy)
    }

    /// Records (or replaces) the registry source of an installed app
    /// (`docs/REGISTRY.md` §9).
    pub fn set_registry(&self, id: &str, name: &str, base: &str) -> Result<()> {
        let mut lib = self.library()?;
        let entry = lib.apps.get_mut(id).with_context(|| format!("{id} is not in the library"))?;
        entry.registry = Some(RegistrySource { name: name.to_owned(), base: base.to_owned() });
        self.save_library(&lib)
    }

    // -- Sources (registries, `docs/REGISTRY.md` §9) ------------------------

    fn sources_store(&self) -> Result<Sources> {
        read_json(&self.sources_path())
    }

    /// The configured registry sources, name -> base.
    pub fn sources(&self) -> Result<BTreeMap<String, String>> {
        Ok(self.sources_store()?.0.into_iter().map(|(name, rec)| (name, rec.base().to_owned())).collect())
    }

    /// Adds a source, or replaces the source of the same name. A replaced
    /// source loses its headers (a new base is a new host).
    pub fn source_add(&self, name: &str, base: &str) -> Result<()> {
        let mut store = self.sources_store()?;
        store.0.insert(name.to_owned(), SourceRecord::Plain(base.to_owned()));
        write_json(&self.sources_path(), &store)
    }

    /// The request headers of source `name` (no values of secrets).
    pub fn source_headers(&self, name: &str) -> Result<Vec<HeaderSpec>> {
        let store = self.sources_store()?;
        Ok(store.0.get(name).with_context(|| format!("no source named `{name}`"))?.headers().to_vec())
    }

    /// Sets the request headers of source `name`, in place of the old ones.
    /// A header has a `value` or a `secret`, not both.
    pub fn source_set_headers(&self, name: &str, headers: Vec<HeaderSpec>) -> Result<()> {
        for h in &headers {
            if h.name.is_empty() || !h.name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                bail!("`{}` is not a valid header name", h.name);
            }
            if h.value.is_some() == h.secret.is_some() {
                bail!("the header `{}` needs a value or a secret, not both and not none", h.name);
            }
        }
        let mut store = self.sources_store()?;
        let rec = store.0.get_mut(name).with_context(|| format!("no source named `{name}`"))?;
        *rec = SourceRecord::new(rec.base(), headers);
        write_json(&self.sources_path(), &store)
    }

    /// The headers to send to the source at `base`, with the secrets read
    /// from the store now. A secret is read at the time of a request and is
    /// not kept. An error names the source and the header, never a value.
    pub fn request_headers(
        &self,
        base: &str,
        secrets: &mut dyn plinth_runner_wasmtime::secrets::SecretsBackend,
    ) -> Result<Vec<(String, String)>> {
        let scope = plinth_runner_wasmtime::secrets::SecretsScope::new(HOST_SECRETS_APP);
        let mut out = Vec::new();
        for (source, rec) in self.sources_store()?.0 {
            if rec.base() != base {
                continue;
            }
            for h in rec.headers() {
                let value = match (&h.value, &h.secret) {
                    (Some(v), None) => v.clone(),
                    (None, Some(secret)) => match secrets.get(&scope, secret) {
                        Ok(Some(v)) => v,
                        Ok(None) => bail!(
                            "the source `{source}` needs the header {}, but its token is not in the secret store (run `plinth hub source set-token {source}`)",
                            h.name
                        ),
                        Err(plinth_runner_wasmtime::secrets::SecretsError::Unsupported) => {
                            bail!("the source `{source}` needs a token, but this computer has no secret store (system keychain)")
                        }
                        Err(plinth_runner_wasmtime::secrets::SecretsError::Failed(why)) => {
                            bail!("the secret store failed for the header {} of source `{source}` ({why})", h.name)
                        }
                    },
                    _ => bail!("the header {} of source `{source}` is not valid", h.name),
                };
                out.push((h.name.clone(), value));
            }
        }
        Ok(out)
    }

    pub fn source_remove(&self, name: &str) -> Result<()> {
        let mut store = self.sources_store()?;
        if store.0.remove(name).is_none() {
            bail!("no source named `{name}`");
        }
        write_json(&self.sources_path(), &store)
    }

    pub fn source_base(&self, name: &str) -> Result<String> {
        self.sources_store()?.0.remove(name).map(|r| r.base().to_owned()).with_context(|| format!("no source named `{name}`"))
    }

    /// The declared capabilities of `id` that still need the user's
    /// decision, in manifest order (`docs/HUB.md` §7.2, §7.3 step 1):
    /// Medium/High capabilities with no grant yet. A `None`/`Low`
    /// capability never appears here: it is granted by default
    /// (`add_package` records that grant; this also treats it as decided
    /// even if, for some reason, no grant was recorded yet).
    pub fn needs_consent(&self, id: &str, declared: &[String]) -> Result<Vec<String>> {
        let grants = self.grants(id)?;
        Ok(undecided(declared, |c| grants.contains_key(c)))
    }

    /// The bytes of one specific installed version of `id` (not
    /// necessarily the active one; used by the update consent flow,
    /// `docs/HUB.md` §7.3 step 3).
    pub fn version_bytes(&self, id: &str, version: &str) -> Result<Vec<u8>> {
        let lib = self.library()?;
        let entry = lib.apps.get(id).with_context(|| format!("{id} is not in the library"))?;
        let v = entry.versions.iter().find(|v| v.version == version).with_context(|| format!("{id} has no version {version}"))?;
        let path = self.packages_dir().join(format!("{}.plnt", v.digest));
        std::fs::read(&path).with_context(|| format!("read {}", path.display()))
    }

    /// Whether every capability in `declared` is decided: granted by
    /// default (`None`/`Low`), or has an explicit grant (allowed or
    /// refused — a refusal is still a decision; the app just does not get
    /// that capability).
    fn capabilities_decided(&self, grants: &BTreeMap<String, Grant>, declared: &[String]) -> bool {
        declared.iter().all(|c| grants.contains_key(c.as_str()) || is_low_risk(c))
    }

    /// The version of `id` that a plain run should use: the pinned
    /// version if one is set, else the newest installed version whose
    /// capabilities are all decided (`docs/HUB.md` §7.3 step 3). Falls
    /// back to the newest version if none qualify (for example the app's
    /// only version has undecided capabilities): the caller is expected
    /// to run consent first in that case.
    pub fn runnable_version(&self, id: &str) -> Result<VersionEntry> {
        let lib = self.library()?;
        let entry = lib.apps.get(id).with_context(|| format!("{id} is not in the library"))?;
        if let Some(pinned) = &entry.pinned
            && let Some(v) = entry.versions.iter().find(|v| &v.version == pinned)
        {
            return Ok(v.clone());
        }
        let grants = self.grants(id)?;
        for v in entry.versions.iter().rev() {
            if self.capabilities_decided(&grants, &v.capabilities) {
                return Ok(v.clone());
            }
        }
        entry.versions.last().cloned().with_context(|| format!("{id} has no versions"))
    }

    // -- Global policy switches (`docs/HUB.md` §7.4) -----------------------

    fn policy_path(&self) -> PathBuf {
        self.dir.join("policy.json")
    }

    fn policy_store(&self) -> Result<GlobalPolicy> {
        read_json(&self.policy_path())
    }

    /// The capabilities no app may use, regardless of grants
    /// (`docs/HUB.md` §7.4, "a global switch can turn off a capability for
    /// all apps").
    pub fn policy_denied(&self) -> Result<Vec<String>> {
        Ok(self.policy_store()?.denied)
    }

    pub fn policy_deny(&self, capability: &str) -> Result<()> {
        let mut store = self.policy_store()?;
        if !store.denied.iter().any(|c| c == capability) {
            store.denied.push(capability.to_owned());
        }
        write_json(&self.policy_path(), &store)
    }

    pub fn policy_allow(&self, capability: &str) -> Result<()> {
        let mut store = self.policy_store()?;
        store.denied.retain(|c| c != capability);
        write_json(&self.policy_path(), &store)
    }

    // -- Trusted Hub keys (`docs/HUB.md` §4.1) ------------------------------

    /// The Hub keys that the user saved in `policy.json`. The environment
    /// variable is not in this list (`trusted_hub_keys_in` merges both).
    pub fn saved_hub_keys(&self) -> Result<Vec<String>> {
        Ok(self.policy_store()?.hub_trusted_keys)
    }

    /// Saves `key` as a trusted Hub key. Only the user may call this, never
    /// code that read a key from a package or a registry without a clear
    /// user action. Gives `true` when the key was new.
    pub fn trust_hub_key(&self, key: &str) -> Result<bool> {
        let key = key.trim();
        validate_hub_key_id(key)?;
        let mut store = self.policy_store()?;
        if store.hub_trusted_keys.iter().any(|k| k == key) {
            return Ok(false);
        }
        store.hub_trusted_keys.push(key.to_owned());
        write_json(&self.policy_path(), &store)?;
        Ok(true)
    }

    /// Removes `key` from the saved Hub keys. Gives `true` when it was there.
    pub fn untrust_hub_key(&self, key: &str) -> Result<bool> {
        let key = key.trim();
        let mut store = self.policy_store()?;
        let before = store.hub_trusted_keys.len();
        store.hub_trusted_keys.retain(|k| k != key);
        if store.hub_trusted_keys.len() == before {
            return Ok(false);
        }
        write_json(&self.policy_path(), &store)?;
        Ok(true)
    }
}

/// Checks that `key` is an `ed25519:` key id (`ed25519:` and the base64url of
/// a 32-byte public key) for `trust_hub_key`.
pub fn validate_hub_key_id(key: &str) -> Result<()> {
    if !key.starts_with("ed25519:") {
        bail!("`{key}` is not a key id; a key id starts with `ed25519:`");
    }
    plinth_package::publisher::parse_key_id(key).map(|_| ())
}

/// The Hub keys that the host trusts: the comma-separated
/// `PLINTH_HUB_TRUSTED_KEYS` first, then the keys saved in `policy.json` in
/// `dir`, with no key twice. It reads the file without making any folder. A
/// file that does not parse adds no key (the host then trusts less, never
/// more).
pub fn trusted_hub_keys_in(dir: &Path) -> Vec<String> {
    let env = std::env::var("PLINTH_HUB_TRUSTED_KEYS").unwrap_or_default();
    let saved = std::fs::read_to_string(dir.join("policy.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<GlobalPolicy>(&t).ok())
        .map(|p| p.hub_trusted_keys)
        .unwrap_or_default();
    merge_hub_keys(&env, saved)
}

/// The keys of the comma-separated `env` and the `saved` keys, in this
/// order, with no empty key and no key twice.
fn merge_hub_keys(env: &str, saved: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for key in env.split(',').map(str::trim).map(str::to_owned).chain(saved) {
        if !key.is_empty() && !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// The hub-wide capability policy (`docs/HUB.md` §7.4), stored in
/// `policy.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GlobalPolicy {
    #[serde(default)]
    denied: Vec<String>,
    /// The data quota of one app in MiB (`Hub::data_quota`); `None` is the default.
    #[serde(default, rename = "dataQuotaMib", skip_serializing_if = "Option::is_none")]
    data_quota_mib: Option<u64>,
    /// The key ids (`ed25519:…`) that the user trusts as Hub keys
    /// (`docs/HUB.md` §4.1). Only an explicit user action adds one.
    #[serde(default, rename = "hubTrustedKeys", skip_serializing_if = "Vec::is_empty")]
    hub_trusted_keys: Vec<String>,
}

/// What `Hub::uninstall` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UninstallReport {
    /// True when the data stayed (`keep_data`).
    pub data_kept: bool,
    /// The bytes of app data that were deleted.
    pub deleted_bytes: u64,
    /// True when the secrets of the app were deleted.
    pub secrets_removed: bool,
    /// The number of grant records that were deleted.
    pub grants_removed: usize,
    /// The number of cached package files that were deleted.
    pub packages_removed: usize,
}

/// The data quota of one app in bytes, for the Hub data directory `dir`
/// (`Hub::data_quota`). It reads `policy.json` without making any folder, so
/// the host can call it every time it starts an app. A value that does not
/// parse or is 0 counts as not set.
pub fn data_quota_in(dir: &Path) -> u64 {
    let mib = |v: u64| (v > 0).then(|| v.saturating_mul(1 << 20));
    if let Some(q) = std::env::var("PLINTH_DATA_QUOTA_MIB").ok().and_then(|v| v.trim().parse::<u64>().ok()).and_then(mib) {
        return q;
    }
    std::fs::read_to_string(dir.join("policy.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<GlobalPolicy>(&t).ok())
        .and_then(|p| p.data_quota_mib)
        .and_then(mib)
        .unwrap_or(DEFAULT_DATA_QUOTA)
}

/// The data quota of one app in bytes for the default Hub directory.
pub fn data_quota() -> u64 {
    data_quota_in(&hub_dir())
}

// -- plinth:hub backend -------------------------------------------------------

/// The native side of `plinth:hub` (`docs/HUB.md` §4.1, §12.2), on top of
/// this Hub's local library. `plinth-host-desktop` boxes one of these into
/// the guest's `Policy`/`Runner` only after it has checked that the
/// package is signed by a trusted Hub key (`docs/HUB.md` §4.1); this type
/// itself does not re-check that, it only answers the calls.
pub struct HubService {
    hub: Hub,
    launches: Vec<String>,
    sources: Option<std::sync::Arc<dyn SourceClient>>,
    uninstall: Option<UninstallSetup>,
}

/// Removes the secrets of an app. The host gives this, because the
/// keychain is not in this crate.
pub type RemoveSecrets = std::sync::Arc<dyn Fn(&str) -> Result<()> + Send + Sync>;

/// What `plinth:hub` `uninstall` needs from the host (`docs/STORAGE.md` §7).
struct UninstallSetup {
    data_dir: PathBuf,
    /// The id of the Hub app that makes the calls. It cannot uninstall itself.
    own_id: String,
    remove_secrets: RemoveSecrets,
}

impl HubService {
    /// A service with no source client: `search` and `install` report that
    /// this host cannot read sources.
    pub fn new(hub: Hub) -> Self {
        Self { hub, launches: Vec::new(), sources: None, uninstall: None }
    }

    /// A service that reads the configured sources (`sources.json`,
    /// `docs/REGISTRY.md` §9) through `client` for `search` and `install`.
    pub fn with_sources(hub: Hub, client: std::sync::Arc<dyn SourceClient>) -> Self {
        Self { hub, launches: Vec::new(), sources: Some(client), uninstall: None }
    }

    /// Lets the service uninstall apps (`docs/STORAGE.md` §7). `data_dir` is
    /// the data directory of the host; `own_id` is the id of the Hub app
    /// that uses this service, which the service refuses to uninstall;
    /// `remove_secrets` deletes the secrets of an app. Without this, the
    /// `uninstall` call fails with a clear error.
    pub fn with_uninstall(mut self, data_dir: PathBuf, own_id: &str, remove_secrets: RemoveSecrets) -> Self {
        self.uninstall = Some(UninstallSetup { data_dir, own_id: own_id.to_owned(), remove_secrets });
        self
    }
}

/// One app that a source lists (`docs/HUB.md` §5.2), for `plinth:hub`'s
/// `search`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub name: String,
    /// The latest version that the source lists.
    pub version: String,
    pub description: String,
}

/// Reads registry sources for the Hub (`docs/REGISTRY.md` §9). The desktop
/// host implements this with `plinth-registry`; tests use a fake. This
/// crate does not depend on `plinth-registry`, so the dependency runs one
/// way.
pub trait SourceClient: Send + Sync {
    /// The apps in the source at `base` that match `query`.
    fn search(&self, base: &str, query: &str) -> Result<Vec<SearchHit>>;
    /// The package bytes of the latest version of `id`, or `None` if the
    /// source does not list `id`.
    fn latest_package(&self, base: &str, id: &str) -> Result<Option<Vec<u8>>>;
    /// The latest version of `id` that the source lists, with the names of
    /// its capabilities, or `None` if the source does not list `id`
    /// (`docs/HUB.md` §9.2). The default says that this client cannot
    /// check for updates.
    fn latest_version(&self, base: &str, id: &str) -> Result<Option<SourceVersion>> {
        let _ = (base, id);
        bail!("this source client cannot check for updates")
    }
    /// The package bytes of `version` of `id`.
    fn package(&self, base: &str, id: &str, version: &str) -> Result<Vec<u8>> {
        let _ = (base, id, version);
        bail!("this source client cannot download a given version")
    }
}

/// One version of an app as a source lists it (`docs/REGISTRY.md` §4).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceVersion {
    pub version: String,
    pub capabilities: Vec<String>,
}

/// Checks the source of each library app (or of `only`) for a newer
/// version than the newest installed one (`docs/HUB.md` §9.2), and records
/// the result in `updates.json`. An app from a file (no source) has no
/// updates. A source that fails adds one line to `errors`. The result is
/// the JSON text that `wit/plinth/app.wit`'s `hub.check-updates`
/// describes: `{ "updates": [{ id, name, current, version, source,
/// newCapabilities, pinned }], "errors": [string] }`.
pub fn check_updates(hub: &Hub, client: &dyn SourceClient, only: Option<&str>) -> Result<String> {
    let mut updates = Vec::new();
    let mut errors = Vec::new();
    for entry in hub.list()? {
        if only.is_some_and(|o| o != entry.id) {
            continue;
        }
        let Some(reg) = &entry.registry else {
            hub.set_available_update(&entry.id, None)?;
            continue;
        };
        let newest = hub.newest_installed(&entry.id)?;
        let current = newest.as_ref().map(|v| v.version.clone()).unwrap_or_default();
        let latest = match client.latest_version(&reg.base, &entry.id) {
            Ok(latest) => latest,
            Err(e) => {
                errors.push(format!("{}: {e:#}", entry.id));
                continue;
            }
        };
        let found = latest.filter(|l| is_newer(&l.version, &current)).map(|l| {
            let old: Vec<String> = newest.as_ref().map(|v| v.capabilities.clone()).unwrap_or_default();
            AvailableUpdate {
                new_capabilities: added_capabilities(&old, &l.capabilities),
                version: l.version,
                source: reg.name.clone(),
            }
        });
        if let Some(u) = &found {
            updates.push(serde_json::json!({
                "id": entry.id,
                "name": entry.name,
                "current": current,
                "version": u.version,
                "source": u.source,
                "newCapabilities": u.new_capabilities,
                "pinned": entry.pinned.clone().unwrap_or_default(),
            }));
        }
        hub.set_available_update(&entry.id, found)?;
    }
    if let Some(only) = only
        && hub.get(only)?.is_none()
    {
        bail!("{only} is not in the library");
    }
    Ok(serde_json::to_string(&serde_json::json!({ "updates": updates, "errors": errors }))?)
}

/// What `apply_update` did.
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    pub id: String,
    pub from: String,
    pub to: String,
    pub source: String,
    /// The capabilities that the new version adds. The Hub asks for the
    /// Medium and High ones before the new version runs (`docs/HUB.md`
    /// §7.3 step 3); until then the previous version runs.
    pub new_capabilities: Vec<String>,
}

/// Installs the latest version of library app `id` from its own source
/// (`docs/HUB.md` §9.2, `docs/REGISTRY.md` §9). `Ok(None)` when it is up to
/// date. The new version runs only after the user decides its new Medium
/// and High capabilities (`Hub::runnable_version`); a pin stays.
pub fn apply_update(hub: &Hub, client: &dyn SourceClient, id: &str) -> Result<Option<Updated>> {
    let entry = hub.get(id)?.with_context(|| format!("{id} is not in the library"))?;
    let reg = entry.registry.clone().with_context(|| format!("{id} was added from a file; it has no source to update from"))?;
    let newest = hub.newest_installed(id)?;
    let current = newest.as_ref().map(|v| v.version.clone()).unwrap_or_default();
    let Some(latest) = client.latest_version(&reg.base, id)? else {
        bail!("the source {} no longer lists {id}", reg.name);
    };
    if !is_newer(&latest.version, &current) {
        hub.set_available_update(id, None)?;
        return Ok(None);
    }
    let bytes = client.package(&reg.base, id, &latest.version)?;
    let pkg = plinth_package::Package::read(&bytes).context("the source gave a package that is not valid")?;
    if pkg.manifest.version != latest.version {
        bail!("source {} gave version {} of {id}, not {}", reg.name, pkg.manifest.version, latest.version);
    }
    let added = hub.add_package(&bytes)?;
    if added != id {
        bail!("source {} gave a package for {added}, not {id}", reg.name);
    }
    hub.set_registry(id, &reg.name, &reg.base)?;
    hub.set_available_update(id, None)?;
    let old: Vec<String> = newest.map(|v| v.capabilities).unwrap_or_default();
    let new_version = hub.get(id)?.and_then(|e| e.versions.into_iter().find(|v| v.version == latest.version));
    let new_capabilities = added_capabilities(&old, &new_version.map(|v| v.capabilities).unwrap_or_default());
    Ok(Some(Updated { id: id.to_owned(), from: current, to: latest.version, source: reg.name, new_capabilities }))
}

/// Searches every configured source of `hub` (`docs/HUB.md` §5.2). A
/// source that fails adds one line to `errors`; the other sources still
/// give their hits. The result is the JSON text that
/// `wit/plinth/app.wit`'s `hub.search` describes.
pub fn search_sources(hub: &Hub, client: &dyn SourceClient, query: &str) -> Result<String> {
    let mut hits = Vec::new();
    let mut errors = Vec::new();
    for (name, base) in hub.sources()? {
        match client.search(&base, query) {
            Ok(found) => {
                for hit in found {
                    hits.push(serde_json::json!({
                        "id": hit.id,
                        "name": hit.name,
                        "version": hit.version,
                        "description": hit.description,
                        "source": name,
                    }));
                }
            }
            Err(e) => errors.push(format!("{name}: {e:#}")),
        }
    }
    Ok(serde_json::to_string(&serde_json::json!({ "hits": hits, "errors": errors }))?)
}

/// Installs the latest version of `id` from the first configured source of
/// `hub` that lists it, and records that source (`docs/REGISTRY.md` §9) so
/// `plinth hub update` finds newer versions. Returns the app id.
pub fn install_from_sources(hub: &Hub, client: &dyn SourceClient, id: &str) -> Result<String> {
    let mut errors = Vec::new();
    for (name, base) in hub.sources()? {
        match client.latest_package(&base, id) {
            Ok(Some(bytes)) => {
                let added = hub.add_package(&bytes)?;
                if added != id {
                    bail!("source {name} gave a package for {added}, not {id}");
                }
                hub.set_registry(&added, &name, &base)?;
                return Ok(added);
            }
            Ok(None) => {}
            Err(e) => errors.push(format!("{name}: {e:#}")),
        }
    }
    if errors.is_empty() {
        bail!("no configured source lists {id}")
    } else {
        bail!("no configured source could give {id} ({})", errors.join("; "))
    }
}

/// `plinth_link::capabilities::Risk` as the lowercase word `docs/HUB.md`
/// §7.2 and `wit/plinth/app.wit`'s `hub.list-apps` doc comment use.
fn risk_word(risk: plinth_link::capabilities::Risk) -> &'static str {
    use plinth_link::capabilities::Risk;
    match risk {
        Risk::None => "none",
        Risk::Low => "low",
        Risk::Medium => "medium",
        Risk::High => "high",
    }
}

impl HubService {
    /// One library app as the JSON object of `hub.list-apps` and
    /// `hub.app-info` (`wit/plinth/app.wit`), or `None` if it has no
    /// versions.
    fn app_json(&self, entry: &LibraryEntry) -> Result<Option<serde_json::Value>, String> {
        let Some(active) = entry.active_version() else { return Ok(None) };
        let report = self.hub.capability_report(&entry.id, &active.capabilities).map_err(|e| e.to_string())?;
        let blocked = self.hub.is_app_blocked(&entry.id).unwrap_or(false);
        let publisher_blocked = self.hub.is_publisher_of_blocked(&entry.id).unwrap_or(false);
        let update = self.hub.available_update(&entry.id).ok().flatten();
        let json = {
            // The manifest gives the publisher name and each capability's
            // reason (`docs/HUB.md` §7.2). A package that cannot be read
            // still lists, without them.
            let manifest = self
                .hub
                .version_bytes(&entry.id, &active.version)
                .ok()
                .and_then(|bytes| plinth_package::Package::read(&bytes).ok())
                .map(|pkg| pkg.manifest);
            let capabilities: Vec<serde_json::Value> = report
                .iter()
                .map(|c| {
                    let description = plinth_link::capabilities::info(&c.name).map(|i| i.description).unwrap_or("");
                    let rationale = manifest
                        .as_ref()
                        .and_then(|m| m.capabilities.iter().find(|d| d.name == c.name))
                        .map(|d| d.rationale.clone())
                        .unwrap_or_default();
                    serde_json::json!({
                        "name": c.name,
                        "risk": risk_word(c.risk),
                        "description": description,
                        "rationale": rationale,
                        "decided": c.decision.is_some(),
                        "allowed": matches!(c.decision, Some(Decision::Allowed)),
                        "byDefault": c.by_default,
                    })
                })
                .collect();
            // Newest first, so a version list reads like a change log.
            let mut versions: Vec<&VersionEntry> = entry.versions.iter().collect();
            versions.sort_by(|a, b| semver_key(&b.version).cmp(&semver_key(&a.version)));
            let versions: Vec<serde_json::Value> = versions
                .into_iter()
                .map(|v| serde_json::json!({ "version": v.version, "signer": v.signer.clone().unwrap_or_default(), "capabilities": v.capabilities }))
                .collect();
            serde_json::json!({
                "id": entry.id,
                "name": entry.name,
                "version": active.version,
                "publisher": manifest.as_ref().map(|m| m.publisher.clone()).unwrap_or_default(),
                "signer": active.signer.clone().unwrap_or_default(),
                "source": entry.registry.as_ref().map(|r| r.name.clone()).unwrap_or_default(),
                "blocked": blocked,
                "publisherBlocked": publisher_blocked,
                "groups": entry.groups,
                "capabilities": capabilities,
                "pinned": entry.pinned.clone().unwrap_or_default(),
                "versions": versions,
                "update": update.as_ref().map(|u| u.version.clone()).unwrap_or_default(),
                "updateCapabilities": update.map(|u| u.new_capabilities).unwrap_or_default(),
            })
        };
        Ok(Some(json))
    }
}

impl plinth_runner_wasmtime::hub::HubBackend for HubService {
    fn list_apps_json(&self) -> Result<String, String> {
        let entries = self.hub.list().map_err(|e| e.to_string())?;
        let mut apps = Vec::new();
        for entry in &entries {
            if let Some(app) = self.app_json(entry)? {
                apps.push(app);
            }
        }
        serde_json::to_string(&apps).map_err(|e| e.to_string())
    }

    fn app_info_json(&self, id: &str) -> Result<String, String> {
        let entry = self.hub.get(id).map_err(|e| e.to_string())?.ok_or_else(|| format!("{id} is not in the library"))?;
        let app = self.app_json(&entry)?.ok_or_else(|| format!("{id} has no versions"))?;
        serde_json::to_string(&app).map_err(|e| e.to_string())
    }

    fn pin(&mut self, id: &str, version: &str) -> Result<(), String> {
        let version = (!version.is_empty()).then(|| version.to_owned());
        self.hub.pin(id, version).map_err(|e| e.to_string())
    }

    fn block_publisher(&mut self, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return Err("an unsigned app has no publisher key to block".to_owned());
        }
        self.hub.block_publisher(key.trim()).map_err(|e| e.to_string())
    }

    fn unblock_publisher(&mut self, key: &str) -> Result<(), String> {
        self.hub.unblock_publisher(key.trim()).map_err(|e| e.to_string())
    }

    fn check_updates(&self, id: &str) -> plinth_runner_wasmtime::hub::HubJob {
        let hub = self.hub.clone();
        let client = self.sources.clone();
        let id = id.to_owned();
        Box::new(move || match client {
            Some(client) => {
                let only = (!id.is_empty()).then_some(id.as_str());
                check_updates(&hub, client.as_ref(), only).map_err(|e| format!("{e:#}"))
            }
            None => Ok(r#"{"updates":[],"errors":["this host cannot read sources"]}"#.to_owned()),
        })
    }

    fn update(&self, id: &str) -> plinth_runner_wasmtime::hub::HubJob {
        let hub = self.hub.clone();
        let client = self.sources.clone();
        let id = id.to_owned();
        Box::new(move || match client {
            Some(client) => match apply_update(&hub, client.as_ref(), &id) {
                Ok(Some(updated)) => Ok(updated.to),
                Ok(None) => Ok(String::new()),
                Err(e) => Err(format!("{e:#}")),
            },
            None => Err("this host cannot read sources".to_owned()),
        })
    }

    fn uninstall(&self, id: &str, keep_data: bool) -> plinth_runner_wasmtime::hub::HubJob {
        let hub = self.hub.clone();
        let id = id.to_owned();
        let setup = self.uninstall.as_ref().map(|s| (s.data_dir.clone(), s.own_id.clone(), s.remove_secrets.clone()));
        Box::new(move || {
            let Some((data_dir, own_id, remove_secrets)) = setup else {
                return Err("this host cannot uninstall apps".to_owned());
            };
            if id == own_id {
                return Err("the Hub cannot uninstall itself".to_owned());
            }
            hub.uninstall(&id, &data_dir, keep_data, &mut |app| remove_secrets(app)).map(|_| String::new()).map_err(|e| format!("{e:#}"))
        })
    }

    fn launch(&mut self, id: &str) {
        self.launches.push(id.to_owned());
    }

    fn take_launches(&mut self) -> Vec<String> {
        std::mem::take(&mut self.launches)
    }

    fn set_grant(&mut self, id: &str, capability: &str, allowed: bool) -> Result<(), String> {
        let decision = if allowed { Decision::Allowed } else { Decision::Refused };
        let version = self
            .hub
            .get(id)
            .map_err(|e| e.to_string())?
            .and_then(|e| e.active_version().cloned())
            .map(|v| v.version)
            .unwrap_or_default();
        self.hub.set_grant(id, capability, decision, &version).map_err(|e| e.to_string())
    }

    fn block(&mut self, id: &str) -> Result<(), String> {
        self.hub.block_app(id).map_err(|e| e.to_string())
    }

    fn unblock(&mut self, id: &str) -> Result<(), String> {
        self.hub.unblock_app(id).map_err(|e| e.to_string())
    }

    fn list_groups_json(&self) -> Result<String, String> {
        let groups = self.hub.groups().map_err(|e| e.to_string())?;
        serde_json::to_string(&groups).map_err(|e| e.to_string())
    }

    fn create_group(&mut self, name: &str) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("a group needs a name".to_owned());
        }
        self.hub.create_group(name.trim()).map_err(|e| e.to_string())
    }

    fn set_group(&mut self, id: &str, group: &str, member: bool) -> Result<(), String> {
        let r = if member { self.hub.add_to_group(id, group) } else { self.hub.remove_from_group(id, group) };
        r.map_err(|e| e.to_string())
    }

    fn remove(&mut self, id: &str) -> Result<(), String> {
        self.hub.remove(id).map_err(|e| e.to_string())
    }

    fn search(&self, query: &str) -> plinth_runner_wasmtime::hub::HubJob {
        let hub = self.hub.clone();
        let client = self.sources.clone();
        let query = query.to_owned();
        Box::new(move || match client {
            Some(client) => search_sources(&hub, client.as_ref(), &query).map_err(|e| format!("{e:#}")),
            None => Ok(r#"{"hits":[],"errors":["this host cannot read sources"]}"#.to_owned()),
        })
    }

    fn install(&self, id: &str) -> plinth_runner_wasmtime::hub::HubJob {
        let hub = self.hub.clone();
        let client = self.sources.clone();
        let id = id.to_owned();
        Box::new(move || match client {
            Some(client) => install_from_sources(&hub, client.as_ref(), &id).map_err(|e| format!("{e:#}")),
            None => Err("this host cannot read sources".to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_hub() -> (Hub, PathBuf) {
        let dir = std::env::temp_dir().join(format!("plinth-hub-test-{}-{}", std::process::id(), rand_suffix()));
        (Hub::open(&dir).unwrap(), dir)
    }

    #[test]
    fn sources_file_keeps_the_old_format_and_adds_headers() {
        let (hub, dir) = temp_hub();
        // A file from an older Hub: name -> base string.
        std::fs::write(hub.sources_path(), r#"{"main":"https://a.example.com","dev":"/tmp/reg"}"#).unwrap();
        assert_eq!(hub.sources().unwrap()["main"], "https://a.example.com");
        assert!(hub.source_headers("main").unwrap().is_empty());
        let spec = HeaderSpec { name: "CF-Access-Client-Id".into(), value: None, secret: Some("source.main.cf-access-client-id".into()) };
        hub.source_set_headers("main", vec![spec.clone()]).unwrap();
        assert_eq!(hub.source_headers("main").unwrap(), vec![spec]);
        assert_eq!(hub.sources().unwrap()["main"], "https://a.example.com");
        // A source with no headers is a plain string again.
        hub.source_set_headers("main", vec![]).unwrap();
        let text = std::fs::read_to_string(hub.sources_path()).unwrap();
        assert!(text.contains(r#""main": "https://a.example.com""#), "{text}");
        // A value and a secret together, or neither, is an error.
        let both = HeaderSpec { name: "X-A".into(), value: Some("v".into()), secret: Some("s".into()) };
        assert!(hub.source_set_headers("main", vec![both]).is_err());
        assert!(hub.source_set_headers("none", vec![]).is_err());
        // A new base drops the headers.
        let lit = HeaderSpec { name: "X-A".into(), value: Some("v".into()), secret: None };
        hub.source_set_headers("main", vec![lit]).unwrap();
        hub.source_add("main", "https://b.example.com").unwrap();
        assert!(hub.source_headers("main").unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    /// A unique suffix for a test directory. The clock alone is not enough:
    /// on Windows two parallel tests can read the same time.
    fn rand_suffix() -> String {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        format!("{t}-{n}")
    }

    /// Compiles `examples/counter` (small, no capabilities) into an app
    /// module and wraps it in a `.plnt` with `id`/`version`. Real app
    /// bytes, so `Hub::add_package`'s link check is exercised for real.
    fn fake_package(id: &str, version: &str) -> Vec<u8> {
        fake_package_with_capabilities(id, version, &[])
    }

    /// Like `fake_package`, but the manifest also declares `caps` (name,
    /// rationale). The counter module itself reaches none of them (it is
    /// still a real, linkable component), so this tests the Hub's grant
    /// and consent bookkeeping, not the compiler's capability check.
    fn fake_package_with_capabilities(id: &str, version: &str, caps: &[(&str, &str)]) -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
        let fs = plinth_compiler::driver::DiskFs { root };
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let artifact = artifact.unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("counter has errors:\n{}", diags.join("\n"))
        });
        let component = artifact.app;
        let mut toml = format!("id = \"{id}\"\nname = \"Test App\"\nversion = \"{version}\"\npublisher = \"me\"\n");
        for (name, rationale) in caps {
            toml.push_str(&format!("\n[[capabilities]]\nname = \"{name}\"\nrationale = \"{rationale}\"\n"));
        }
        let cfg = plinth_package::ProjectConfig::parse(&toml).unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
        let pkg = plinth_package::Package::new(manifest, component, Vec::new());
        pkg.write().unwrap()
    }

    /// Like `fake_package`, but signed with `identity` (`docs/HUB.md`
    /// §6.1, phase H1). `identity.name` becomes the manifest's publisher.
    fn fake_signed_package(id: &str, version: &str, identity: &plinth_package::publisher::PublisherIdentity) -> Vec<u8> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
        let fs = plinth_compiler::driver::DiskFs { root };
        let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
        let artifact = artifact.unwrap_or_else(|| {
            let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
            panic!("counter has errors:\n{}", diags.join("\n"))
        });
        let component = artifact.app;
        let toml = format!("id = \"{id}\"\nname = \"Test App\"\nversion = \"{version}\"\npublisher = \"{}\"\n", identity.name);
        let cfg = plinth_package::ProjectConfig::parse(&toml).unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &component);
        let mut pkg = plinth_package::Package::new(manifest, component, Vec::new());
        pkg.signature = Some(plinth_package::signature::sign(&pkg, identity).unwrap());
        pkg.write().unwrap()
    }

    fn fake_identity(name: &str) -> plinth_package::publisher::PublisherIdentity {
        plinth_package::publisher::PublisherIdentity {
            name: name.to_owned(),
            signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng),
        }
    }

    /// §6.1: a signed package records its signer key, and a publisher
    /// block refuses every one of its apps.
    #[test]
    fn signed_package_records_signer_and_publisher_block() {
        let (hub, dir) = temp_hub();
        let identity = fake_identity("Acme");
        let id = hub.add_package(&fake_signed_package("com.example.notes", "0.1.0", &identity)).unwrap();
        let entry = hub.get(&id).unwrap().unwrap();
        assert_eq!(entry.versions[0].signer.as_deref(), Some(identity.key_id().as_str()));

        hub.block_publisher(&identity.key_id()).unwrap();
        assert!(hub.add_package(&fake_signed_package("com.other.app", "0.1.0", &identity)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §6.1: a new version signed by a different key than the installed
    /// version is refused (rotation is future work, `docs/HUB.md` §6.1).
    #[test]
    fn key_change_is_refused() {
        let (hub, dir) = temp_hub();
        let acme = fake_identity("Acme");
        let other = fake_identity("Acme");
        let id = hub.add_package(&fake_signed_package("com.example.notes", "0.1.0", &acme)).unwrap();
        assert_eq!(id, "com.example.notes");
        let err = hub.add_package(&fake_signed_package("com.example.notes", "0.2.0", &other)).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("different key"), "{err}");
        assert!(text.contains("plinth hub remove com.example.notes"), "{err}");
        assert!(text.contains("plinth hub uninstall com.example.notes"), "{err}");
        assert!(text.contains("data of the old version stays on disk"), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_list_remove_package() {
        let (hub, dir) = temp_hub();
        let bytes = fake_package("com.example.notes", "0.1.0");
        let id = hub.add_package(&bytes).unwrap();
        assert_eq!(id, "com.example.notes");
        let apps = hub.list().unwrap();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].versions.len(), 1);
        let back = hub.package(&id).unwrap();
        assert_eq!(back, bytes);
        hub.remove(&id).unwrap();
        assert!(hub.list().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_package_in_a_newer_format_needs_a_newer_plinth() {
        use std::io::Write as _;
        let (hub, dir) = temp_hub();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("plinth-profile", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(b"plinth/2").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        // The plain message, not "not a valid .plnt package".
        let err = hub.add_package(&bytes).unwrap_err();
        assert_eq!(err.to_string(), "This app needs a newer version of Plinth (package format plinth/2)");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn add_second_version_appends() {
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package("com.example.notes", "0.1.0")).unwrap();
        hub.add_package(&fake_package("com.example.notes", "0.2.0")).unwrap();
        let entry = hub.get(&id).unwrap().unwrap();
        assert_eq!(entry.versions.len(), 2);
        assert_eq!(entry.active_version().unwrap().version, "0.2.0");
        hub.pin(&id, Some("0.1.0".into())).unwrap();
        assert_eq!(hub.get(&id).unwrap().unwrap().active_version().unwrap().version, "0.1.0");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grants_round_trip_and_policy() {
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package("com.example.notes", "0.1.0")).unwrap();
        let declared = vec!["store.kv".to_string(), "clipboard.read".to_string()];
        // store.kv is Low risk: never needs consent. clipboard.read is
        // Medium: it does, until decided.
        assert_eq!(hub.needs_consent(&id, &declared).unwrap(), vec!["clipboard.read".to_string()]);
        hub.set_grant(&id, "store.kv", Decision::Allowed, "0.1.0").unwrap();
        hub.set_grant(&id, "clipboard.read", Decision::Refused, "0.1.0").unwrap();
        assert!(hub.needs_consent(&id, &declared).unwrap().is_empty());
        let policy = hub.policy_for(&id, &declared).unwrap();
        assert_eq!(policy.check("store.kv"), Ok(()));
        assert_eq!(policy.check("clipboard.read"), Err(plinth_runner_wasmtime::policy::DeniedReason::Refused));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §7.2: a Low-risk declared capability is granted "by default" the
    /// moment `add_package` installs it, with no consent step and no
    /// explicit grant call; the consent window only ever needs to ask for
    /// Medium/High ones. A Low one can still be refused later.
    #[test]
    fn low_risk_granted_by_default_at_install() {
        let (hub, dir) = temp_hub();
        let id = hub
            .add_package(&fake_package_with_capabilities(
                "com.example.notes",
                "0.1.0",
                &[("store.kv", "save notes"), ("clipboard.read", "paste")],
            ))
            .unwrap();
        // Only the Medium one needs consent.
        assert_eq!(hub.needs_consent(&id, &["store.kv".into(), "clipboard.read".into()]).unwrap(), vec!["clipboard.read".to_string()]);
        let report = hub.capability_report(&id, &["store.kv".into(), "clipboard.read".into()]).unwrap();
        assert_eq!(report[0].decision, Some(Decision::Allowed));
        assert!(report[0].by_default);
        assert_eq!(report[1].decision, None);
        assert!(!report[1].by_default);
        // The app can run with store.kv already allowed.
        let policy = hub.policy_for(&id, &["store.kv".into()]).unwrap();
        assert_eq!(policy.check("store.kv"), Ok(()));
        // The user can still refuse a Low-risk capability explicitly.
        hub.set_grant(&id, "store.kv", Decision::Refused, "0.1.0").unwrap();
        let policy = hub.policy_for(&id, &["store.kv".into()]).unwrap();
        assert_eq!(policy.check("store.kv"), Err(plinth_runner_wasmtime::policy::DeniedReason::Refused));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §7.3 step 3: a new version that adds a capability only asks consent
    /// for the new one; if the user cancels (never decides it), the
    /// previous version is the one `runnable_version`/`package` picks.
    #[test]
    fn update_asks_only_for_new_capability_and_falls_back() {
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package_with_capabilities("com.example.notes", "0.1.0", &[("store.kv", "save notes")])).unwrap();
        // 0.1.0 is fully decided (store.kv is Low).
        assert_eq!(hub.runnable_version(&id).unwrap().version, "0.1.0");

        hub.add_package(&fake_package_with_capabilities(
            "com.example.notes",
            "0.2.0",
            &[("store.kv", "save notes"), ("clipboard.read", "paste")],
        ))
        .unwrap();
        let v2_caps = vec!["store.kv".to_string(), "clipboard.read".to_string()];
        // Only the new capability needs a decision.
        assert_eq!(hub.needs_consent(&id, &v2_caps).unwrap(), vec!["clipboard.read".to_string()]);
        // Cancelling (never deciding clipboard.read): the previous,
        // fully-decided version is what `package`/`runnable_version` run.
        assert_eq!(hub.runnable_version(&id).unwrap().version, "0.1.0");
        let bytes = hub.package(&id).unwrap();
        assert_eq!(bytes, hub.version_bytes(&id, "0.1.0").unwrap());

        // Deciding clipboard.read makes 0.2.0 runnable again.
        hub.set_grant(&id, "clipboard.read", Decision::Allowed, "0.2.0").unwrap();
        assert_eq!(hub.runnable_version(&id).unwrap().version, "0.2.0");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// §7.4: a global deny overrides even an earlier "allowed" grant.
    #[test]
    fn global_policy_overrides_grant() {
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package_with_capabilities("com.example.notes", "0.1.0", &[("clipboard.read", "paste")])).unwrap();
        hub.set_grant(&id, "clipboard.read", Decision::Allowed, "0.1.0").unwrap();
        let policy = hub.policy_for(&id, &["clipboard.read".into()]).unwrap();
        assert_eq!(policy.check("clipboard.read"), Ok(()));
        hub.policy_deny("clipboard.read").unwrap();
        assert_eq!(hub.policy_denied().unwrap(), vec!["clipboard.read".to_string()]);
        let policy = hub.policy_for(&id, &["clipboard.read".into()]).unwrap();
        assert_eq!(policy.check("clipboard.read"), Err(plinth_runner_wasmtime::policy::DeniedReason::Refused));
        hub.policy_allow("clipboard.read").unwrap();
        let policy = hub.policy_for(&id, &["clipboard.read".into()]).unwrap();
        assert_eq!(policy.check("clipboard.read"), Ok(()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn block_prevents_add_and_run() {
        let (hub, dir) = temp_hub();
        hub.block_app("com.example.notes").unwrap();
        assert!(hub.is_blocked("com.example.notes").unwrap());
        assert!(hub.add_package(&fake_package("com.example.notes", "0.1.0")).is_err());
        hub.unblock_app("com.example.notes").unwrap();
        assert!(!hub.is_blocked("com.example.notes").unwrap());
        hub.add_package(&fake_package("com.example.notes", "0.1.0")).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn groups() {
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package("com.example.notes", "0.1.0")).unwrap();
        hub.add_to_group(&id, "Work").unwrap();
        assert_eq!(hub.groups().unwrap(), vec!["Work".to_string()]);
        assert_eq!(hub.get(&id).unwrap().unwrap().groups, vec!["Work".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `docs/HUB.md` §4.1, §12.2: the `plinth:hub` backend's `list-apps`
    /// reports the library, including a capability's risk and whether it
    /// is granted, and `launch`/`block`/`unblock`/`set-grant` act on it.
    #[test]
    fn hub_service_lists_launches_and_manages_apps() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package_with_capabilities("com.example.notes", "0.1.0", &[("clipboard.read", "paste")])).unwrap();
        let mut svc = HubService::new(hub.clone());

        let json = svc.list_apps_json().unwrap();
        let apps: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(apps[0]["id"], id);
        assert_eq!(apps[0]["blocked"], false);
        assert_eq!(apps[0]["capabilities"][0]["name"], "clipboard.read");
        assert_eq!(apps[0]["capabilities"][0]["risk"], "medium");
        assert_eq!(apps[0]["capabilities"][0]["allowed"], false);

        svc.set_grant(&id, "clipboard.read", true).unwrap();
        let apps: serde_json::Value = serde_json::from_str(&svc.list_apps_json().unwrap()).unwrap();
        assert_eq!(apps[0]["capabilities"][0]["allowed"], true);

        svc.launch(&id);
        assert_eq!(svc.take_launches(), vec![id.clone()]);
        assert!(svc.take_launches().is_empty());

        svc.block(&id).unwrap();
        let apps: serde_json::Value = serde_json::from_str(&svc.list_apps_json().unwrap()).unwrap();
        assert_eq!(apps[0]["blocked"], true);
        svc.unblock(&id).unwrap();

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The capability label fields of `list-apps` (`docs/HUB.md` §7.2):
    /// the fixed description, the app's reason, and who decided.
    #[test]
    fn list_apps_has_the_capability_label_and_groups() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        let id = hub
            .add_package(&fake_package_with_capabilities("com.example.notes", "0.1.0", &[("store.kv", "save notes"), ("clipboard.read", "paste")]))
            .unwrap();
        let mut svc = HubService::new(hub.clone());
        svc.set_group(&id, "Work", true).unwrap();
        let apps: serde_json::Value = serde_json::from_str(&svc.list_apps_json().unwrap()).unwrap();
        let app = &apps[0];
        assert_eq!(app["groups"], serde_json::json!(["Work"]));
        assert_eq!(app["source"], "");
        let caps = app["capabilities"].as_array().unwrap();
        let kv = caps.iter().find(|c| c["name"] == "store.kv").unwrap();
        assert_eq!(kv["risk"], "low");
        assert_eq!(kv["description"], "save data on this device");
        assert_eq!(kv["rationale"], "save notes");
        assert_eq!(kv["decided"], true);
        assert_eq!(kv["byDefault"], true);
        let clip = caps.iter().find(|c| c["name"] == "clipboard.read").unwrap();
        assert_eq!(clip["decided"], false);
        assert_eq!(clip["allowed"], false);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `docs/HUB.md` §9.1: groups through the `plinth:hub` backend, and
    /// `remove`.
    #[test]
    fn hub_service_groups_and_remove() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        let id = hub.add_package(&fake_package("com.example.notes", "0.1.0")).unwrap();
        let mut svc = HubService::new(hub.clone());
        assert_eq!(svc.list_groups_json().unwrap(), "[]");
        svc.create_group("Family").unwrap();
        assert!(svc.create_group("  ").is_err());
        svc.set_group(&id, "Work", true).unwrap();
        assert_eq!(svc.list_groups_json().unwrap(), r#"["Family","Work"]"#);
        assert_eq!(hub.get(&id).unwrap().unwrap().groups, vec!["Work".to_string()]);
        svc.set_group(&id, "Work", false).unwrap();
        assert!(hub.get(&id).unwrap().unwrap().groups.is_empty());
        // The group stays when its last app leaves it.
        assert_eq!(svc.list_groups_json().unwrap(), r#"["Family","Work"]"#);
        svc.remove(&id).unwrap();
        assert!(hub.get(&id).unwrap().is_none());
        assert!(svc.remove(&id).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A fake source client: `good` lists `com.example.notes` with the
    /// versions in `packages` (the last one is the latest); `bad` always
    /// fails.
    struct FakeSources {
        package: Vec<u8>,
        /// (version, capabilities, package bytes), oldest first.
        packages: std::sync::Mutex<Vec<(String, Vec<String>, Vec<u8>)>>,
    }

    impl FakeSources {
        fn new(package: Vec<u8>) -> Self {
            let pkg = plinth_package::Package::read(&package).unwrap();
            let caps = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
            Self { packages: std::sync::Mutex::new(vec![(pkg.manifest.version.clone(), caps, package.clone())]), package }
        }

        fn publish(&self, package: Vec<u8>) {
            let pkg = plinth_package::Package::read(&package).unwrap();
            let caps = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
            self.packages.lock().unwrap().push((pkg.manifest.version.clone(), caps, package));
        }
    }

    impl SourceClient for FakeSources {
        fn latest_version(&self, base: &str, id: &str) -> Result<Option<SourceVersion>> {
            if base == "bad" {
                bail!("cannot reach the source");
            }
            if id != "com.example.notes" {
                return Ok(None);
            }
            let packages = self.packages.lock().unwrap();
            Ok(packages.last().map(|(version, capabilities, _)| SourceVersion { version: version.clone(), capabilities: capabilities.clone() }))
        }

        fn package(&self, _base: &str, _id: &str, version: &str) -> Result<Vec<u8>> {
            let packages = self.packages.lock().unwrap();
            packages.iter().find(|p| p.0 == version).map(|p| p.2.clone()).with_context(|| format!("no version {version}"))
        }

        fn search(&self, base: &str, query: &str) -> Result<Vec<SearchHit>> {
            if base == "bad" {
                bail!("cannot reach the source");
            }
            let hit = SearchHit { id: "com.example.notes".into(), name: "Notes".into(), version: "0.1.0".into(), description: "Take notes".into() };
            Ok(if "notes".contains(&query.to_lowercase()) { vec![hit] } else { Vec::new() })
        }

        fn latest_package(&self, base: &str, id: &str) -> Result<Option<Vec<u8>>> {
            if base == "bad" {
                bail!("cannot reach the source");
            }
            Ok((id == "com.example.notes").then(|| self.package.clone()))
        }
    }

    /// `docs/HUB.md` §5.2: search across every source (one failing source
    /// does not hide the others), then install from the first source that
    /// lists the app, recording that source.
    #[test]
    fn hub_service_searches_and_installs_from_sources() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        hub.source_add("a-broken", "bad").unwrap();
        hub.source_add("main", "good").unwrap();
        let client = std::sync::Arc::new(FakeSources::new(fake_package("com.example.notes", "0.1.0")));
        let svc = HubService::with_sources(hub.clone(), client);

        let result: serde_json::Value = serde_json::from_str(&(svc.search("note"))().unwrap()).unwrap();
        assert_eq!(result["hits"][0]["id"], "com.example.notes");
        assert_eq!(result["hits"][0]["source"], "main");
        assert_eq!(result["errors"].as_array().unwrap().len(), 1);
        assert!(result["errors"][0].as_str().unwrap().starts_with("a-broken:"));

        assert_eq!((svc.install("com.example.notes"))().unwrap(), "com.example.notes");
        let entry = hub.get("com.example.notes").unwrap().unwrap();
        assert_eq!(entry.registry.unwrap().name, "main");

        let err = (svc.install("com.example.missing"))().unwrap_err();
        assert!(err.contains("no configured source could give"), "{err}");

        // Without a source client, search answers with one error.
        let plain = HubService::new(hub.clone());
        let result: serde_json::Value = serde_json::from_str(&(plain.search("x"))().unwrap()).unwrap();
        assert_eq!(result["errors"][0], "this host cannot read sources");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `docs/HUB.md` §9.2, §7.3 step 3: the Hub finds a newer version in the
    /// app's source, shows it in `list-apps`, installs it, and runs it only
    /// after the user decides its new Medium capability. A pin holds the
    /// older version.
    #[test]
    fn check_and_apply_updates_with_reconsent_and_pins() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        hub.source_add("main", "good").unwrap();
        let v1 = fake_package_with_capabilities("com.example.notes", "0.1.0", &[("store.kv", "save notes")]);
        let client = std::sync::Arc::new(FakeSources::new(v1));
        let mut svc = HubService::with_sources(hub.clone(), client.clone());
        (svc.install("com.example.notes"))().unwrap();
        // An app from a file has no source: never an update.
        hub.add_package(&fake_package("com.example.local", "1.0.0")).unwrap();

        // Up to date.
        let result: serde_json::Value = serde_json::from_str(&(svc.check_updates(""))().unwrap()).unwrap();
        assert_eq!(result["updates"], serde_json::json!([]));

        // The source publishes 0.2.0, which adds clipboard.read (Medium).
        client.publish(fake_package_with_capabilities(
            "com.example.notes",
            "0.2.0",
            &[("store.kv", "save notes"), ("clipboard.read", "paste")],
        ));
        let result: serde_json::Value = serde_json::from_str(&(svc.check_updates(""))().unwrap()).unwrap();
        assert_eq!(result["updates"][0]["id"], "com.example.notes");
        assert_eq!(result["updates"][0]["current"], "0.1.0");
        assert_eq!(result["updates"][0]["version"], "0.2.0");
        assert_eq!(result["updates"][0]["newCapabilities"], serde_json::json!(["clipboard.read"]));
        assert_eq!(result["updates"].as_array().unwrap().len(), 1);
        let info: serde_json::Value = serde_json::from_str(&svc.app_info_json("com.example.notes").unwrap()).unwrap();
        assert_eq!(info["update"], "0.2.0");
        assert_eq!(info["updateCapabilities"], serde_json::json!(["clipboard.read"]));
        assert!(svc.app_info_json("com.example.missing").is_err());
        // Only one app: a check for an unknown id is an error.
        assert!((svc.check_updates("com.example.missing"))().is_err());

        // Apply: 0.2.0 is installed, but 0.1.0 runs until clipboard.read is decided.
        assert_eq!((svc.update("com.example.notes"))().unwrap(), "0.2.0");
        assert_eq!(hub.available_update("com.example.notes").unwrap(), None);
        assert_eq!(hub.runnable_version("com.example.notes").unwrap().version, "0.1.0");
        assert_eq!(hub.needs_consent("com.example.notes", &["store.kv".into(), "clipboard.read".into()]).unwrap(), vec!["clipboard.read"]);
        let info: serde_json::Value = serde_json::from_str(&svc.app_info_json("com.example.notes").unwrap()).unwrap();
        assert_eq!(info["version"], "0.2.0");
        assert_eq!(info["update"], "");
        assert_eq!(info["versions"][0]["version"], "0.2.0");
        assert_eq!(info["versions"][1]["version"], "0.1.0");
        let clip = info["capabilities"].as_array().unwrap().iter().find(|c| c["name"] == "clipboard.read").unwrap().clone();
        assert_eq!(clip["decided"], false);
        svc.set_grant("com.example.notes", "clipboard.read", true).unwrap();
        assert_eq!(hub.runnable_version("com.example.notes").unwrap().version, "0.2.0");
        // A second update finds nothing.
        assert_eq!((svc.update("com.example.notes"))().unwrap(), "");
        assert!((svc.update("com.example.local"))().unwrap_err().contains("added from a file"));

        // Pin and unpin.
        svc.pin("com.example.notes", "0.1.0").unwrap();
        assert_eq!(hub.runnable_version("com.example.notes").unwrap().version, "0.1.0");
        let info: serde_json::Value = serde_json::from_str(&svc.app_info_json("com.example.notes").unwrap()).unwrap();
        assert_eq!(info["pinned"], "0.1.0");
        assert_eq!(info["version"], "0.1.0");
        assert!(svc.pin("com.example.notes", "9.9.9").is_err());
        svc.pin("com.example.notes", "").unwrap();
        assert_eq!(hub.runnable_version("com.example.notes").unwrap().version, "0.2.0");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `docs/HUB.md` §7.4: a publisher block stops every installed app of
    /// that key, and `list-apps` tells it apart from an app block.
    #[test]
    fn publisher_block_through_the_service() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        let acme = fake_identity("Acme");
        let id = hub.add_package(&fake_signed_package("com.example.notes", "0.1.0", &acme)).unwrap();
        let mut svc = HubService::new(hub.clone());
        assert!(svc.block_publisher("").is_err());
        svc.block_publisher(&acme.key_id()).unwrap();
        assert!(hub.is_blocked(&id).unwrap());
        assert!(!hub.is_app_blocked(&id).unwrap());
        let info: serde_json::Value = serde_json::from_str(&svc.app_info_json(&id).unwrap()).unwrap();
        assert_eq!(info["blocked"], false);
        assert_eq!(info["publisherBlocked"], true);
        // A new version of the same publisher cannot be added either.
        assert!(hub.add_package(&fake_signed_package("com.example.notes", "0.2.0", &acme)).is_err());
        svc.unblock_publisher(&acme.key_id()).unwrap();
        assert!(!hub.is_blocked(&id).unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `docs/HUB.md` §10: the app icon becomes `icons/<id>.ico` in the Hub
    /// directory, for a `.lnk` shortcut.
    #[test]
    fn writes_the_app_icon_as_an_ico_file() {
        let (hub, dir) = temp_hub();
        let mut pkg = plinth_package::Package::read(&fake_package("com.example.notes", "0.1.0")).unwrap();
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&64u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0]);
        pkg.manifest.icon = Some("assets/icon.png".into());
        pkg.assets.push(("assets/icon.png".into(), png.clone()));
        let id = hub.add_package(&pkg.write().unwrap()).unwrap();
        let path = hub.write_app_icon(&id).unwrap().unwrap();
        assert_eq!(path, dir.join("icons").join("com.example.notes.ico"));
        let ico = std::fs::read(&path).unwrap();
        assert_eq!(&ico[22..], &png[..]);
        // No icon: no file.
        let other = hub.add_package(&fake_package("com.example.plain", "0.1.0")).unwrap();
        assert_eq!(hub.write_app_icon(&other).unwrap(), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Writes sample data for `id` under `data` and returns its layout.
    fn put_app_data(hub: &Hub, id: &str, data: &Path) -> appdata::AppDataLayout {
        let layout = hub.data_layout(id, data).unwrap();
        std::fs::create_dir_all(&layout.kv).unwrap();
        std::fs::write(layout.kv.join("kv.json"), r#"{"k":"v"}"#).unwrap();
        for space in &layout.spaces {
            std::fs::create_dir_all(space.files.join("d")).unwrap();
            std::fs::write(space.files.join("d/f.txt"), "file").unwrap();
            std::fs::create_dir_all(&space.database).unwrap();
            std::fs::write(space.database.join("store.sqlite"), "not really a database").unwrap();
        }
        layout
    }

    /// S13: uninstall with data deletes everything the app owns, the
    /// secrets callback runs, the grants and the cached packages go, and
    /// another app is not touched.
    #[test]
    fn uninstall_deletes_the_app_and_its_data() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let id = hub.add_package(&fake_package_with_capabilities("com.example.gone", "0.1.0", &[("store.kv", "k")])).unwrap();
        let keep = hub.add_package(&fake_package_with_capabilities("com.example.stays", "0.1.0", &[("store.kv", "k")])).unwrap();
        let layout = put_app_data(&hub, &id, &data);
        let other = put_app_data(&hub, &keep, &data);
        assert!(!hub.grants(&id).unwrap().is_empty());
        let digest = hub.get(&id).unwrap().unwrap().versions[0].digest.clone();
        let package_file = hub.packages_dir().join(format!("{digest}.plnt"));
        assert!(package_file.exists());
        hub.block_app("com.example.blocked").unwrap();

        let mut secrets_for = Vec::new();
        let report = hub
            .uninstall(&id, &data, false, &mut |app| {
                secrets_for.push(app.to_owned());
                Ok(())
            })
            .unwrap();
        assert_eq!(secrets_for, vec![id.clone()]);
        assert!(report.secrets_removed && !report.data_kept);
        assert!(report.deleted_bytes > 0 && report.grants_removed >= 1 && report.packages_removed == 1);
        assert!(!layout.exists());
        assert!(hub.get(&id).unwrap().is_none());
        assert!(hub.grants(&id).unwrap().is_empty());
        assert!(!package_file.exists());
        assert!(other.exists() && hub.get(&keep).unwrap().is_some() && !hub.grants(&keep).unwrap().is_empty());
        assert!(hub.is_app_blocked("com.example.blocked").unwrap(), "the block list is not changed");
        // The app is gone, so a second uninstall says so.
        assert!(hub.uninstall(&id, &data, false, &mut |_| Ok(())).unwrap_err().to_string().contains("not in the library"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--keep-data`: the app goes, the data and the grants stay, the secrets
    /// callback does not run. A reinstall of the same package finds the data.
    #[test]
    fn uninstall_can_keep_the_data() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let bytes = fake_package("com.example.keeper", "0.1.0");
        let id = hub.add_package(&bytes).unwrap();
        let layout = put_app_data(&hub, &id, &data);
        let report = hub.uninstall(&id, &data, true, &mut |_| panic!("the secrets stay")).unwrap();
        assert!(report.data_kept && !report.secrets_removed && report.deleted_bytes == 0);
        assert!(hub.get(&id).unwrap().is_none());
        assert!(layout.exists());
        hub.add_package(&bytes).unwrap();
        assert_eq!(hub.data_layout(&id, &data).unwrap().spaces, layout.spaces, "the same package gets the same space");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A running app (a shared run lock) stops the uninstall before any change.
    #[test]
    fn uninstall_refuses_while_the_app_runs() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let id = hub.add_package(&fake_package("com.example.busy", "0.1.0")).unwrap();
        let layout = put_app_data(&hub, &id, &data);
        let running = RunLock::running(&data, &id).unwrap();
        let err = hub.uninstall(&id, &data, false, &mut |_| panic!("nothing changes while it runs")).unwrap_err().to_string();
        assert!(err.contains("is running") && err.contains("close"), "{err}");
        assert!(layout.exists() && hub.get(&id).unwrap().is_some());
        drop(running);
        hub.uninstall(&id, &data, false, &mut |_| Ok(())).unwrap();
        assert!(!layout.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A failing secrets removal leaves the app in the library, so the user
    /// can run the command again.
    #[test]
    fn uninstall_keeps_the_library_entry_when_the_secrets_fail() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let id = hub.add_package(&fake_package("com.example.fails", "0.1.0")).unwrap();
        put_app_data(&hub, &id, &data);
        let err = hub.uninstall(&id, &data, false, &mut |_| bail!("keychain locked")).unwrap_err();
        assert!(format!("{err:#}").contains("keychain locked"));
        assert!(hub.get(&id).unwrap().is_some());
        hub.uninstall(&id, &data, false, &mut |_| Ok(())).unwrap();
        assert!(hub.get(&id).unwrap().is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `plinth:hub` `uninstall` (`HubService`): data and secrets go unless `keep_data`; an app that
    /// runs, an unknown app and the Hub app itself give an error and change nothing.
    #[test]
    fn the_hub_service_uninstalls_with_or_without_the_data() {
        use plinth_runner_wasmtime::hub::HubBackend;
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let hub_app = hub.add_package(&fake_package("com.example.hub", "0.1.0")).unwrap();
        let id = hub.add_package(&fake_package("com.example.victim", "0.1.0")).unwrap();
        let keep = hub.add_package(&fake_package("com.example.keeper", "0.1.0")).unwrap();
        let layout = put_app_data(&hub, &id, &data);
        let keep_layout = put_app_data(&hub, &keep, &data);
        let secrets: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let log = secrets.clone();
        let svc = HubService::new(hub.clone()).with_uninstall(
            data.clone(),
            &hub_app,
            std::sync::Arc::new(move |app| {
                log.lock().unwrap().push(app.to_owned());
                Ok(())
            }),
        );

        // The Hub app itself: refused before anything is touched.
        let err = svc.uninstall(&hub_app, false)().unwrap_err();
        assert!(err.contains("cannot uninstall itself"), "{err}");
        assert!(hub.get(&hub_app).unwrap().is_some());
        // An unknown app.
        assert!(svc.uninstall("com.example.nope", false)().unwrap_err().contains("not in the library"));
        // A running app: the error says to close it; nothing changes.
        let running = RunLock::running(&data, &id).unwrap();
        let err = svc.uninstall(&id, false)().unwrap_err();
        assert!(err.contains("is running") && err.contains("close"), "{err}");
        assert!(layout.exists() && hub.get(&id).unwrap().is_some() && secrets.lock().unwrap().is_empty());
        drop(running);

        // With the data.
        svc.uninstall(&id, false)().unwrap();
        assert!(!layout.exists() && hub.get(&id).unwrap().is_none());
        assert_eq!(*secrets.lock().unwrap(), vec![id.clone()]);
        // Keeping the data: the app goes, the data and the secrets stay.
        svc.uninstall(&keep, true)().unwrap();
        assert!(keep_layout.exists() && hub.get(&keep).unwrap().is_none());
        assert_eq!(secrets.lock().unwrap().len(), 1);

        // A service the host did not set up for uninstall says so.
        let plain = HubService::new(hub.clone());
        assert!(plain.uninstall(&hub_app, true)().unwrap_err().contains("cannot uninstall apps"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Ids that point outside the app's folders never delete anything.
    #[test]
    fn uninstall_refuses_ids_that_look_like_paths() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let victim = dir.join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("precious.txt"), "x").unwrap();
        std::fs::create_dir_all(data.join("apps")).unwrap();
        for bad in ["..", "../victim", "..\\victim", "a/b", "", ".", HOST_SECRETS_APP] {
            let err = hub.uninstall(bad, &data, false, &mut |_| panic!("no secrets for {bad:?}")).unwrap_err().to_string();
            assert!(err.contains("not a valid app id") || err.contains("not an app") || err.contains("not in the library"), "{bad:?}: {err}");
            assert!(hub.export_data(bad, &data, &dir.join("x.zip")).is_err(), "{bad:?}");
        }
        assert!(victim.join("precious.txt").exists() && data.join("apps").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two versions of an unsigned app have two spaces; uninstall deletes both.
    #[test]
    fn uninstall_deletes_every_space_of_an_unsigned_app() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let id = hub.add_package(&fake_package("com.example.twice", "0.1.0")).unwrap();
        hub.add_package(&fake_package("com.example.twice", "0.2.0")).unwrap();
        let layout = put_app_data(&hub, &id, &data);
        assert_eq!(layout.spaces.len(), 2);
        hub.uninstall(&id, &data, false, &mut |_| Ok(())).unwrap();
        assert!(!layout.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Export through the Hub names the newest version in the manifest.
    #[test]
    fn export_data_writes_a_zip_with_the_version() {
        let (hub, dir) = temp_hub();
        let data = dir.join("data");
        let id = hub.add_package(&fake_package("com.example.export", "0.3.0")).unwrap();
        let layout = hub.data_layout(&id, &data).unwrap();
        std::fs::create_dir_all(&layout.kv).unwrap();
        std::fs::write(layout.kv.join("kv.json"), r#"{"k":"v"}"#).unwrap();
        std::fs::create_dir_all(&layout.spaces[0].files).unwrap();
        std::fs::write(layout.spaces[0].files.join("f.txt"), "file").unwrap();
        let out = dir.join("out").join("export.zip");
        let report = hub.export_data(&id, &data, &out).unwrap();
        assert_eq!((report.kv_entries, report.files, report.databases), (Some(1), 1, 0));
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("manifest.json").unwrap(), &mut text).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(manifest["version"], "0.3.0");
        assert_eq!(manifest["app"], "com.example.export");
        // The data stays after an export.
        assert!(layout.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_data_quota_comes_from_the_policy_file() {
        let (hub, dir) = temp_hub();
        // PLINTH_DATA_QUOTA_MIB is read too, so this test does not set or
        // rely on the environment: it only runs when the variable is unset.
        if std::env::var_os("PLINTH_DATA_QUOTA_MIB").is_some() {
            return;
        }
        assert_eq!(hub.data_quota(), DEFAULT_DATA_QUOTA);
        assert_eq!(DEFAULT_DATA_QUOTA, 100 << 20);
        hub.policy_deny("net").unwrap();
        hub.set_data_quota_mib(Some(5)).unwrap();
        assert_eq!(hub.data_quota(), 5 << 20);
        assert_eq!(hub.policy_denied().unwrap(), vec!["net".to_string()], "the deny list stays");
        assert!(hub.set_data_quota_mib(Some(0)).is_err());
        std::fs::write(dir.join("policy.json"), r#"{"dataQuotaMib":0}"#).unwrap();
        assert_eq!(hub.data_quota(), DEFAULT_DATA_QUOTA, "0 counts as not set");
        std::fs::write(dir.join("policy.json"), "nonsense").unwrap();
        assert_eq!(hub.data_quota(), DEFAULT_DATA_QUOTA);
        hub.set_data_quota_mib(None).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A valid key id, from a fixed signing key.
    fn test_key(seed: u8) -> String {
        plinth_package::publisher::key_id(&ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key())
    }

    #[test]
    fn hub_keys_are_validated() {
        assert!(validate_hub_key_id(&test_key(1)).is_ok());
        for bad in ["", "ed25519:", "ed25519:abc", "rsa:AAAA", "AAAA", "ed25519:!!!!", " ed25519:x"] {
            assert!(validate_hub_key_id(bad).is_err(), "{bad:?}");
        }
        let (hub, dir) = temp_hub();
        assert!(hub.trust_hub_key("ed25519:nope").is_err());
        assert!(!dir.join("policy.json").exists(), "a bad key writes nothing");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hub_keys_are_saved_in_the_policy_file_and_removed() {
        let (hub, dir) = temp_hub();
        let (a, b) = (test_key(1), test_key(2));
        assert!(hub.saved_hub_keys().unwrap().is_empty());
        assert!(hub.trust_hub_key(&a).unwrap(), "new");
        assert!(!hub.trust_hub_key(&format!(" {a} ")).unwrap(), "known (and the space is cut)");
        assert!(hub.trust_hub_key(&b).unwrap());
        assert_eq!(hub.saved_hub_keys().unwrap(), vec![a.clone(), b.clone()]);
        // The file has the other policy values too, and no temp file stays.
        hub.policy_deny("net").unwrap();
        hub.set_data_quota_mib(Some(5)).unwrap();
        assert_eq!(hub.saved_hub_keys().unwrap().len(), 2, "other policy writes keep the keys");
        assert_eq!(hub.policy_denied().unwrap(), vec!["net".to_string()]);
        let text = std::fs::read_to_string(dir.join("policy.json")).unwrap();
        assert!(text.contains("hubTrustedKeys") && text.contains(&a), "{text}");
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".tmp-")));
        // The host reads the same file (the env var may add keys, never remove).
        let trusted = trusted_hub_keys_in(&dir);
        assert!(trusted.contains(&a) && trusted.contains(&b));
        assert!(hub.untrust_hub_key(&a).unwrap());
        assert!(!hub.untrust_hub_key(&a).unwrap(), "not there any more");
        assert_eq!(hub.saved_hub_keys().unwrap(), vec![b.clone()]);
        assert!(!trusted_hub_keys_in(&dir).contains(&a));
        // A file that does not parse gives no trusted key.
        std::fs::write(dir.join("policy.json"), "nonsense").unwrap();
        assert!(!trusted_hub_keys_in(&dir).contains(&b));
        // No hub folder at all: nothing, and nothing is made.
        assert!(!trusted_hub_keys_in(&dir.join("missing")).contains(&b));
        assert!(!dir.join("missing").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hub_keys_of_the_env_var_and_the_file_merge_without_doubles() {
        let merged = merge_hub_keys(" ed25519:a, ,ed25519:b,ed25519:a", vec!["ed25519:b".into(), "ed25519:c".into(), String::new()]);
        assert_eq!(merged, vec!["ed25519:a", "ed25519:b", "ed25519:c"]);
        assert!(merge_hub_keys("", vec![]).is_empty());
    }

    #[test]
    fn versions_compare_as_semver() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(is_newer("1.0.1", ""));
    }
}
