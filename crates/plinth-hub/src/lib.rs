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

/// The registry sources known to this Hub (`hub source add/list/remove`),
/// stored in `sources.json`: name -> base URL/path.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Sources(pub BTreeMap<String, String>);

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
fn is_low_risk(capability: &str) -> bool {
    matches!(risk_of(capability), plinth_link::capabilities::Risk::None | plinth_link::capabilities::Risk::Low)
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
/// directly; the CLI and the desktop host use `hub_dir()`.
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
    fn sources_path(&self) -> PathBuf {
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
        let pkg = plinth_package::Package::read(bytes).context("not a valid .plnt package")?;
        plinth_link::cores::link_app(&pkg.component).context("the package does not link against an installed core")?;
        if self.is_blocked(&pkg.manifest.id)? {
            bail!("{} is blocked", pkg.manifest.id);
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
        entry.name = pkg.manifest.name.clone();
        let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
        if !entry.versions.iter().any(|v| v.version == pkg.manifest.version) {
            entry.versions.push(VersionEntry { version: pkg.manifest.version.clone(), digest, capabilities: declared.clone() });
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
        self.save_library(&lib)
    }

    /// The package bytes for the app's active version (`docs/HUB.md`
    /// §9.1).
    pub fn package(&self, id: &str) -> Result<Vec<u8>> {
        let version = self.runnable_version(id)?;
        let path = self.packages_dir().join(format!("{}.plnt", version.digest));
        std::fs::read(&path).with_context(|| format!("read {}", path.display()))
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

    pub fn is_blocked(&self, id: &str) -> Result<bool> {
        let blocks = self.blocks_store()?;
        if blocks.apps.iter().any(|a| a == id) {
            return Ok(true);
        }
        // Publisher blocks need the manifest's publisher; checked by the
        // caller with `is_publisher_blocked` once it has the manifest (no
        // signed publisher ids exist until H1, so this list is usually
        // empty today).
        Ok(false)
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
            match grants.get(cap) {
                Some(Grant { decision: Decision::Allowed, .. }) => {}
                Some(Grant { decision: Decision::Refused, .. }) => policy.refuse(cap.clone()),
                None if is_low_risk(cap) => {}
                None => policy.refuse(cap.clone()),
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
        Ok(self.sources_store()?.0)
    }

    pub fn source_add(&self, name: &str, base: &str) -> Result<()> {
        let mut store = self.sources_store()?;
        store.0.insert(name.to_owned(), base.to_owned());
        write_json(&self.sources_path(), &store)
    }

    pub fn source_remove(&self, name: &str) -> Result<()> {
        let mut store = self.sources_store()?;
        if store.0.remove(name).is_none() {
            bail!("no source named `{name}`");
        }
        write_json(&self.sources_path(), &store)
    }

    pub fn source_base(&self, name: &str) -> Result<String> {
        self.sources_store()?.0.remove(name).with_context(|| format!("no source named `{name}`"))
    }

    /// The declared capabilities of `id` that still need the user's
    /// decision, in manifest order (`docs/HUB.md` §7.2, §7.3 step 1):
    /// Medium/High capabilities with no grant yet. A `None`/`Low`
    /// capability never appears here: it is granted by default
    /// (`add_package` records that grant; this also treats it as decided
    /// even if, for some reason, no grant was recorded yet).
    pub fn needs_consent(&self, id: &str, declared: &[String]) -> Result<Vec<String>> {
        let grants = self.grants(id)?;
        Ok(declared.iter().filter(|c| !grants.contains_key(c.as_str()) && !is_low_risk(c)).cloned().collect())
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
}

/// The hub-wide capability policy (`docs/HUB.md` §7.4), stored in
/// `policy.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GlobalPolicy {
    denied: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_hub() -> (Hub, PathBuf) {
        let dir = std::env::temp_dir().join(format!("plinth-hub-test-{}-{}", std::process::id(), rand_suffix()));
        (Hub::open(&dir).unwrap(), dir)
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
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
        let pkg = plinth_package::Package { manifest, component, assets: Vec::new() };
        pkg.write().unwrap()
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
}
