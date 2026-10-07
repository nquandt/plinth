//! The `.plnt` package (SPEC.md §10).
//!
//! A package is a zip archive:
//!
//! ```text
//! plinth-profile   one line: "plinth/1"
//! manifest.toml
//! app.wasm         the component
//! assets/…
//! ```
//!
//! The reader rejects path traversal, absolute paths, duplicate entries,
//! unknown top-level entries and archives that expand too much. Opening a
//! package grants nothing.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::io::{Cursor, Read, Write};

pub mod feed;
pub mod publisher;
pub mod signature;

pub const PROFILE: &str = "plinth/1";
pub const EXTENSION: &str = "plnt";
/// The runtime version that this build of the tools targets.
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The limits that stop zip bombs.
const MAX_ENTRY: u64 = 64 << 20;
const MAX_TOTAL: u64 = 256 << 20;
const MAX_ENTRIES: usize = 4096;

/// `plinth.toml`: what the author writes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ProjectConfig {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// The author's core policy (SPEC.md §10.5, `docs/UPDATES.md` §4
    /// item 2). An old `plinth.toml` with no `core` field means `follow`,
    /// today's only behavior.
    #[serde(default)]
    pub core: CorePolicy,
    /// The author's update feed (`docs/UPDATES.md` §8 step 2). `None`
    /// means the app never checks for updates (today's behavior, and the
    /// behavior of a `plinth.toml` written before this field existed).
    #[serde(default)]
    pub updates: Option<UpdatesConfig>,
}

/// `[updates]` in `plinth.toml`, carried into the manifest's `updates`
/// field unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct UpdatesConfig {
    /// The feed URL, for example `https://example.com/myapp/feed.json`.
    pub feed: String,
    #[serde(default)]
    pub channel: Channel,
    #[serde(default)]
    pub check: CheckPolicy,
    #[serde(default)]
    pub apply: ApplyPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }
}

/// When the host checks the feed. Only `on-start` is implemented
/// (`docs/UPDATES.md` §8 step 4); an interval is future work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckPolicy {
    #[default]
    OnStart,
}

/// When a downloaded update takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApplyPolicy {
    /// Run the new version the next time the app starts (implemented).
    #[default]
    NextStart,
    /// Ask the user to restart now (implemented as a stub prompt;
    /// `docs/UPDATES.md` §8 step 6).
    Prompt,
    /// Replace the running app in place, keeping its state
    /// (`docs/UPDATES.md` §9 UP-Q2). Not implemented: a config that asks
    /// for it is rejected by `ProjectConfig::validate`.
    Live,
}

/// The author's policy for which core a built app takes, carried into the
/// package manifest's `core` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CorePolicy {
    /// The newest compatible minor and patch version (today's behavior).
    #[default]
    Follow,
    /// Only patches of the minor version the app was built with.
    Patch,
    /// The exact core build the app was built with, by digest.
    Pinned,
}

impl CorePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CorePolicy::Follow => "follow",
            CorePolicy::Patch => "patch",
            CorePolicy::Pinned => "pinned",
        }
    }

    /// Parses a manifest's `core` field. An unrecognized value (from a
    /// newer author tool) falls back to `follow`, the safe default.
    pub fn parse(s: &str) -> CorePolicy {
        match s {
            "patch" => CorePolicy::Patch,
            "pinned" => CorePolicy::Pinned,
            _ => CorePolicy::Follow,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub name: String,
    pub rationale: String,
}

/// `manifest.toml` inside a package (SPEC.md §10.2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub ui_api: String,
    pub runtime: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The SHA-256 of the entry module, in hex.
    pub digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<Capability>,
    /// The author's core policy, `"follow"`, `"patch"` or `"pinned"`
    /// (SPEC.md §10.5). A manifest written before this field existed has
    /// none, which also means `follow`.
    #[serde(default = "default_core_field")]
    pub core: String,
    /// For `core = "pinned"`: the SHA-256 digest, in hex, of the core
    /// build the app was built with. `None` for the other policies, and
    /// for a manifest written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_digest: Option<String>,
    /// The author's update feed configuration (`docs/UPDATES.md` §8 step
    /// 2), carried from `plinth.toml`'s `[updates]` unchanged. `None`
    /// means no updates: a manifest written before this field existed,
    /// or a `plinth.toml` with no `[updates]` section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdatesConfig>,
}

fn default_core_field() -> String {
    CorePolicy::Follow.as_str().to_owned()
}

impl ProjectConfig {
    pub fn parse(text: &str) -> Result<Self> {
        let c: ProjectConfig = toml::from_str(text).context("plinth.toml is not valid")?;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> Result<()> {
        let id_ok = self.id.split('.').count() >= 2
            && self.id.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        if !id_ok {
            bail!("plinth.toml: `id` must be a reverse domain name, for example \"com.example.notes\"");
        }
        if self.name.trim().is_empty() {
            bail!("plinth.toml: `name` must not be empty");
        }
        if !is_semver(&self.version) {
            bail!("plinth.toml: `version` must be MAJOR.MINOR.PATCH");
        }
        for c in &self.capabilities {
            if c.rationale.trim().is_empty() {
                bail!("plinth.toml: the capability `{}` needs a `rationale` (SPEC.md §11)", c.name);
            }
        }
        if let Some(updates) = &self.updates {
            if updates.feed.trim().is_empty() {
                bail!("plinth.toml: `[updates] feed` must not be empty");
            }
            if updates.apply == ApplyPolicy::Live {
                bail!("plinth.toml: `[updates] apply = \"live\"` is not built yet (docs/UPDATES.md §9 UP-Q2); use \"next-start\" or \"prompt\"");
            }
        }
        Ok(())
    }

    /// The manifest for a built app module. `runtime` is the id of the
    /// runtime build that the app needs (SPEC.md §10.2). Equivalent to
    /// `manifest_with_core_digest` with no core digest; use that instead
    /// when the policy is `pinned` (SPEC.md §10.5).
    pub fn manifest(&self, ui_api: &str, runtime: &str, accent: Option<String>, app: &[u8]) -> Manifest {
        self.manifest_with_core_digest(ui_api, runtime, accent, app, None)
    }

    /// Like `manifest`, and records `core_digest` (the SHA-256 digest, in
    /// hex, of the core used to build the app) when the policy is
    /// `pinned`. Ignored for the other policies.
    pub fn manifest_with_core_digest(&self, ui_api: &str, runtime: &str, accent: Option<String>, app: &[u8], core_digest: Option<&str>) -> Manifest {
        Manifest {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            publisher: self.publisher.clone(),
            ui_api: ui_api.to_owned(),
            runtime: runtime.to_owned(),
            entry: "app.wasm".into(),
            accent,
            icon: self.icon.clone(),
            digest: hex(&Sha256::digest(app)),
            capabilities: self.capabilities.clone(),
            updates: self.updates.clone(),
            core: self.core.as_str().to_owned(),
            core_digest: match self.core {
                CorePolicy::Pinned => core_digest.map(str::to_owned),
                _ => None,
            },
        }
    }
}

fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// `MAJOR.MINOR.PATCH` as a comparable tuple; a part that does not parse
/// counts as 0 (the same rule as `plinth_hub::semver_key` and
/// `plinth_registry::is_newer`).
pub fn semver_key(v: &str) -> (u64, u64, u64) {
    let mut it = v.split(['.', '-', '+']).map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// True if version `a` is newer than version `b` (semver order).
pub fn is_newer(a: &str, b: &str) -> bool {
    semver_key(a) > semver_key(b)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The canonical text of a manifest: the bytes that `Package::write` puts
/// in `manifest.toml` and that `signature::sign` digests for a package
/// that was not read from a file. Both use this one function, so the
/// bytes in the file are always the bytes that were signed.
pub fn manifest_toml(manifest: &Manifest) -> Result<String> {
    Ok(toml::to_string(manifest)?)
}

/// A package in memory.
///
/// Make a new package with `Package::new`. `Package::read` also keeps the
/// raw bytes of `manifest.toml` and `plinth-profile`, and the entries that
/// the host does not use (`source/…`), so that `write` gives them back
/// unchanged and `signature::verify` digests the bytes of the file, never
/// a new serialization.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub component: Vec<u8>,
    pub assets: Vec<(String, Vec<u8>)>,
    /// The raw bytes of `signature.json`, if the package carries one
    /// (`docs/HUB.md` §6.1, `signature` module). `None` for an unsigned
    /// package.
    pub signature: Option<Vec<u8>>,
    /// The raw bytes of `plinth-profile`.
    profile: Vec<u8>,
    /// For a package from `read`: the manifest as parsed, and the raw
    /// bytes of `manifest.toml`. The raw bytes are used while `manifest`
    /// is equal to the parsed value.
    read_manifest: Option<(Manifest, Vec<u8>)>,
    /// The entries that the host does not use (`source/…`). They are kept
    /// and signed, but never served as assets and never run.
    other: Vec<(String, Vec<u8>)>,
}

impl Package {
    /// A new package with no signature and no other entries.
    pub fn new(manifest: Manifest, component: Vec<u8>, assets: Vec<(String, Vec<u8>)>) -> Package {
        Package { manifest, component, assets, signature: None, profile: PROFILE.as_bytes().to_vec(), read_manifest: None, other: Vec::new() }
    }

    /// The bytes of `manifest.toml`: the raw bytes from the file if the
    /// manifest did not change after `read`, else `manifest_toml`. `write`
    /// and `signature::sign`/`verify` all use this function.
    pub fn manifest_bytes(&self) -> Result<Cow<'_, [u8]>> {
        match &self.read_manifest {
            Some((parsed, raw)) if *parsed == self.manifest => Ok(Cow::Borrowed(raw)),
            _ => Ok(Cow::Owned(manifest_toml(&self.manifest)?.into_bytes())),
        }
    }

    /// The raw bytes of `plinth-profile`.
    pub fn profile_bytes(&self) -> &[u8] {
        &self.profile
    }

    /// The entries that the host does not use. The host never serves them
    /// and never runs them.
    pub fn other_entries(&self) -> &[(String, Vec<u8>)] {
        &self.other
    }

    /// Adds an entry that the host does not use (for example `source/…`).
    pub fn add_other_entry(&mut self, path: String, bytes: Vec<u8>) -> Result<()> {
        check_path(&path)?;
        if is_reserved_entry(&path) || path.starts_with("assets/") || path == self.manifest.entry {
            bail!("`{path}` cannot be an other entry");
        }
        self.other.push((path, bytes));
        Ok(())
    }

    /// Every entry except `signature.json`, as (path, bytes): the entries
    /// that the signature covers (`signature` module).
    pub fn signed_entries(&self) -> Result<Vec<(String, Cow<'_, [u8]>)>> {
        let mut out = vec![
            ("plinth-profile".to_owned(), Cow::Borrowed(&self.profile[..])),
            ("manifest.toml".to_owned(), self.manifest_bytes()?),
            (self.manifest.entry.clone(), Cow::Borrowed(&self.component[..])),
        ];
        for (path, bytes) in self.assets.iter().chain(&self.other) {
            out.push((path.clone(), Cow::Borrowed(&bytes[..])));
        }
        Ok(out)
    }

    pub fn write(&self) -> Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        check_entry_name(&self.manifest.entry)?;
        zip.start_file("plinth-profile", stored)?;
        zip.write_all(&self.profile)?;
        zip.start_file("manifest.toml", opts)?;
        zip.write_all(&self.manifest_bytes()?)?;
        zip.start_file(self.manifest.entry.as_str(), opts)?;
        zip.write_all(&self.component)?;
        for (path, bytes) in &self.assets {
            check_path(path)?;
            if !path.starts_with("assets/") {
                bail!("asset `{path}` must be under assets/");
            }
            zip.start_file(path.as_str(), opts)?;
            zip.write_all(bytes)?;
        }
        for (path, bytes) in &self.other {
            check_path(path)?;
            zip.start_file(path.as_str(), opts)?;
            zip.write_all(bytes)?;
        }
        if let Some(sig) = &self.signature {
            zip.start_file("signature.json", opts)?;
            zip.write_all(sig)?;
        }
        Ok(zip.finish()?.into_inner())
    }

    /// Reads and validates a package.
    pub fn read(bytes: &[u8]) -> Result<Package> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("the file is not a .plnt package")?;
        if zip.len() > MAX_ENTRIES {
            bail!("the package has too many entries");
        }
        let mut seen = std::collections::HashSet::new();
        let mut total = 0u64;
        let (mut profile, mut manifest, mut files) = (None, None, Vec::new());
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name().to_owned();
            if f.is_dir() {
                continue;
            }
            check_path(&name)?;
            if f.is_symlink() {
                bail!("the package entry `{name}` is a symlink");
            }
            if !seen.insert(name.clone()) {
                bail!("the package has the entry `{name}` two times");
            }
            if f.size() > MAX_ENTRY {
                bail!("the package entry `{name}` is too large");
            }
            total += f.size();
            if total > MAX_TOTAL {
                bail!("the package expands to more than {} MiB", MAX_TOTAL >> 20);
            }
            let mut data = Vec::with_capacity(f.size() as usize);
            // `take` enforces the limit even if the size header lies.
            (&mut f).take(MAX_ENTRY + 1).read_to_end(&mut data)?;
            if data.len() as u64 > MAX_ENTRY {
                bail!("the package entry `{name}` is too large");
            }
            match name.as_str() {
                "plinth-profile" => profile = Some(data),
                "manifest.toml" => manifest = Some(data),
                n if n == "app.wasm" || n.starts_with("assets/") || n.starts_with("source/") || n == "signature.json" => {
                    files.push((name, data))
                }
                other => bail!("the package has an unknown entry `{other}`"),
            }
        }
        let profile = profile.context("the package has no plinth-profile")?;
        if String::from_utf8_lossy(&profile).trim() != PROFILE {
            bail!("the package profile is not `{PROFILE}`");
        }
        let manifest_raw = manifest.context("the package has no manifest.toml")?;
        let manifest: Manifest = toml::from_str(std::str::from_utf8(&manifest_raw).context("manifest.toml is not UTF-8")?)
            .context("manifest.toml is not valid")?;
        check_entry_name(&manifest.entry)?;
        let pos = files.iter().position(|(n, _)| *n == manifest.entry).context("the manifest entry is not in the package")?;
        let component = files.remove(pos).1;
        if hex(&Sha256::digest(&component)) != manifest.digest {
            bail!("the digest of `{}` does not match the manifest", manifest.entry);
        }
        let sig_pos = files.iter().position(|(n, _)| n == "signature.json");
        let signature = sig_pos.map(|i| files.remove(i).1);
        let (assets, other) = files.into_iter().partition(|(n, _)| n.starts_with("assets/"));
        Ok(Package { read_manifest: Some((manifest.clone(), manifest_raw)), manifest, component, assets, signature, profile, other })
    }
}

/// The entries with a fixed meaning that are not app files.
fn is_reserved_entry(path: &str) -> bool {
    matches!(path, "plinth-profile" | "manifest.toml" | "signature.json")
}

/// The manifest's `entry` must be a safe path that is not a reserved entry
/// and not an asset.
fn check_entry_name(entry: &str) -> Result<()> {
    check_path(entry)?;
    if is_reserved_entry(entry) || entry.starts_with("assets/") {
        bail!("the manifest entry `{entry}` is not allowed");
    }
    Ok(())
}

fn check_path(p: &str) -> Result<()> {
    let bad = p.is_empty()
        || p.starts_with('/')
        || p.starts_with('\\')
        || p.contains('\\')
        || p.contains(':')
        || p.split('/').any(|s| s == ".." || s == "." || s.is_empty());
    if bad {
        bail!("the package path `{p}` is not allowed");
    }
    Ok(())
}

/// True if the bytes look like a `.plnt` (zip) file.
pub fn is_package(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

// -- Single-file export (SPEC.md §10.3) ---------------------------------------

/// The last bytes of a single-file export: the magic `PLNTH\0` and the
/// footer format version.
pub const PAYLOAD_MAGIC: [u8; 8] = *b"PLNTH\0\x01\0";
/// The footer: the SHA-256 of the payload, its offset and its length (both
/// little-endian `u64`), and `PAYLOAD_MAGIC`.
const FOOTER_LEN: usize = 32 + 8 + 8 + 8;

/// Adds a `.plnt` to the end of a host executable.
pub fn append_payload(host: &[u8], plnt: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(host.len() + plnt.len() + FOOTER_LEN);
    out.extend_from_slice(host);
    let offset = out.len() as u64;
    out.extend_from_slice(plnt);
    out.extend_from_slice(&Sha256::digest(plnt));
    out.extend_from_slice(&offset.to_le_bytes());
    out.extend_from_slice(&(plnt.len() as u64).to_le_bytes());
    out.extend_from_slice(&PAYLOAD_MAGIC);
    out
}

/// Reads the `.plnt` payload of a single-file export, if the file has one.
/// It reads only the footer and the payload, not the whole executable.
pub fn read_payload(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let size = f.metadata()?.len();
    if size < FOOTER_LEN as u64 {
        return Ok(None);
    }
    let mut footer = [0u8; FOOTER_LEN];
    f.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    f.read_exact(&mut footer)?;
    if footer[48..] != PAYLOAD_MAGIC {
        return Ok(None);
    }
    let offset = u64::from_le_bytes(footer[32..40].try_into().unwrap());
    let len = u64::from_le_bytes(footer[40..48].try_into().unwrap());
    if len > MAX_TOTAL || offset.checked_add(len).is_none_or(|end| end > size - FOOTER_LEN as u64) {
        bail!("the payload footer of {} is not valid", path.display());
    }
    let mut payload = vec![0u8; len as usize];
    f.seek(SeekFrom::Start(offset))?;
    f.read_exact(&mut payload)?;
    if Sha256::digest(&payload)[..] != footer[..32] {
        bail!("the payload of {} does not match its digest", path.display());
    }
    Ok(Some(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProjectConfig {
        ProjectConfig::parse("id = \"dev.plinth.test\"\nname = \"Test\"\nversion = \"0.1.0\"\npublisher = \"me\"\n").unwrap()
    }

    #[test]
    fn payload_round_trip() {
        let dir = std::env::temp_dir().join(format!("plinth-payload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("host.exe");
        std::fs::write(&plain, b"MZ host bytes").unwrap();
        assert!(read_payload(&plain).unwrap().is_none());
        let exe = dir.join("app.exe");
        std::fs::write(&exe, append_payload(b"MZ host bytes", b"PK\x03\x04 a package")).unwrap();
        assert_eq!(read_payload(&exe).unwrap().as_deref(), Some(&b"PK\x03\x04 a package"[..]));
        // A changed payload fails the digest check.
        let mut bytes = std::fs::read(&exe).unwrap();
        bytes[15] ^= 1;
        std::fs::write(&exe, bytes).unwrap();
        assert!(read_payload(&exe).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trip() {
        let wasm = b"\0asm fake component".to_vec();
        let pkg = Package::new(
            config().manifest("1.0", "plinth-rt/test", Some("teal".into()), &wasm),
            wasm.clone(),
            vec![("assets/a.txt".into(), b"hi".to_vec())],
        );
        let bytes = pkg.write().unwrap();
        assert!(is_package(&bytes));
        let back = Package::read(&bytes).unwrap();
        assert_eq!(back.manifest, pkg.manifest);
        assert_eq!(back.component, wasm);
        assert_eq!(back.assets.len(), 1);
        // The bytes in the file are the canonical text of the manifest.
        assert_eq!(&*back.manifest_bytes().unwrap(), manifest_toml(&pkg.manifest).unwrap().as_bytes());
        // A change after `read` gives the new canonical text.
        let mut changed = back.clone();
        changed.manifest.version = "2.0.0".into();
        assert_eq!(&*changed.manifest_bytes().unwrap(), manifest_toml(&changed.manifest).unwrap().as_bytes());
    }

    #[test]
    fn rejects_bad_paths_and_digests() {
        for p in ["../x", "/abs", "a/../b", "C:/x", "a\\b", "a//b"] {
            assert!(check_path(p).is_err(), "{p}");
        }
        let wasm = b"abc".to_vec();
        let mut pkg = Package::new(config().manifest("1.0", "plinth-rt/test", None, &wasm), wasm, Vec::new());
        pkg.component = b"tampered".to_vec();
        assert!(Package::read(&pkg.write().unwrap()).is_err());
    }

    #[test]
    fn validates_config() {
        assert!(ProjectConfig::parse("id = \"x\"\nname = \"T\"\nversion = \"0.1.0\"\npublisher = \"me\"\n").is_err());
        assert!(ProjectConfig::parse("id = \"a.b\"\nname = \"T\"\nversion = \"1\"\npublisher = \"me\"\n").is_err());
        assert!(ProjectConfig::parse("id = \"a.b\"\nname = \"T\"\nversion = \"0.1.0\"\npublisher = \"me\"\nextra = 1\n").is_err());
    }

    #[test]
    fn an_old_config_with_no_core_field_means_follow() {
        let cfg = config();
        assert_eq!(cfg.core, CorePolicy::Follow);
        assert_eq!(cfg.manifest("1.0", "plinth-rt/test", None, b"abc").core, "follow");
    }

    #[test]
    fn core_policy_round_trips_through_plinth_toml() {
        let text = "id = \"dev.plinth.test\"\nname = \"Test\"\nversion = \"0.1.0\"\npublisher = \"me\"\ncore = \"pinned\"\n";
        let cfg = ProjectConfig::parse(text).unwrap();
        assert_eq!(cfg.core, CorePolicy::Pinned);
        // The digest shows up in the manifest only for `pinned`.
        let m = cfg.manifest_with_core_digest("1.0", "plinth-rt/test", None, b"abc", Some("deadbeef"));
        assert_eq!(m.core, "pinned");
        assert_eq!(m.core_digest, Some("deadbeef".into()));
        // A `follow` or `patch` manifest never carries a digest, even if given one.
        let follow = config().manifest_with_core_digest("1.0", "plinth-rt/test", None, b"abc", Some("deadbeef"));
        assert_eq!(follow.core, "follow");
        assert_eq!(follow.core_digest, None);
    }

    #[test]
    fn an_unknown_core_value_in_plinth_toml_is_rejected() {
        let text = "id = \"dev.plinth.test\"\nname = \"Test\"\nversion = \"0.1.0\"\npublisher = \"me\"\ncore = \"nightly\"\n";
        assert!(ProjectConfig::parse(text).is_err());
    }

    #[test]
    fn an_old_manifest_toml_with_no_core_field_parses_as_follow() {
        let toml = r#"
            id = "dev.plinth.test"
            name = "Test"
            version = "0.1.0"
            publisher = "me"
            ui-api = "1.0"
            runtime = "plinth-rt/test"
            entry = "app.wasm"
            digest = "abc"
        "#;
        let m: Manifest = toml::from_str(toml).unwrap();
        assert_eq!(m.core, "follow");
        assert_eq!(m.core_digest, None);
    }
}
