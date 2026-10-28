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
//! The reader rejects path traversal, absolute paths, duplicate entries and
//! archives that expand too much. It keeps an entry that it does not know
//! (`Package::other_entries`), but never runs it and never serves it as an
//! asset; the signature covers it. A newer `plinth-profile` gives the
//! error `NewerFormat` (`docs/COMPATIBILITY.md` §4). Opening a package
//! grants nothing.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::io::{Cursor, Read, Write};

pub mod feed;
pub mod publisher;
pub mod signature;
pub mod theme;

pub const PROFILE: &str = "plinth/1";
/// The newest container format version that this build reads.
pub const PROFILE_VERSION: u32 = 1;

/// The error for a package in a container format that is newer than this
/// build reads (`docs/COMPATIBILITY.md` §4). It is not "invalid package":
/// a newer Plinth reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewerFormat {
    /// The profile of the package, for example `plinth/2`.
    pub profile: String,
}

impl std::fmt::Display for NewerFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "This app needs a newer version of Plinth (package format {})", self.profile)
    }
}

impl std::error::Error for NewerFormat {}

/// Adds `context` (for example "not a valid .plnt package") to an error of
/// `Package::read`, except to a `NewerFormat` error: that package is not
/// invalid, and its message stands alone.
pub fn read_error(e: anyhow::Error, context: impl std::fmt::Display + Send + Sync + 'static) -> anyhow::Error {
    if e.is::<NewerFormat>() { e } else { e.context(context) }
}

/// Checks the bytes of `plinth-profile`: `plinth/N` with `N` from 1 to
/// `PROFILE_VERSION` is accepted, a larger `N` gives `NewerFormat`, and
/// any other text is an error.
fn check_profile(raw: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(raw).unwrap_or("").trim();
    let version = text
        .strip_prefix("plinth/")
        .filter(|n| !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0'))
        .and_then(|n| n.parse::<u32>().ok());
    match version {
        Some(n) if (1..=PROFILE_VERSION).contains(&n) => Ok(()),
        Some(n) => Err(NewerFormat { profile: format!("plinth/{n}") }.into()),
        None => bail!("the package profile is not valid (expected `{PROFILE}`)"),
    }
}
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
    /// `[theme]`: the app's values for the host color tokens
    /// (`docs/ui.md`, "Theming an app"). `None`: the host theme.
    #[serde(default)]
    pub theme: Option<theme::ThemeOverride>,
}

/// `[updates]` in `plinth.toml`, carried into the manifest's `updates`
/// field unchanged.
///
/// The type is lenient, for the manifest (`docs/COMPATIBILITY.md` §4): it
/// ignores a field that it does not know, and an enum value that it does
/// not know maps to the default of the field. `ProjectConfig::parse` keeps
/// `plinth.toml` strict with `check_strict`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
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

impl UpdatesConfig {
    /// The fields of `[updates]`, for the strict check of `plinth.toml`.
    const FIELDS: &[&str] = &["feed", "channel", "check", "apply"];
}

/// A lenient `Deserialize` for an enum of `[updates]`: a value that this
/// version does not know (from a newer author tool) maps to the default of
/// the field, and the host logs it (`docs/COMPATIBILITY.md` §4).
macro_rules! lenient_enum {
    ($ty:ident, $field:literal) => {
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Ok($ty::from_name(&s).unwrap_or_else(|| {
                    log::warn!("manifest.toml: `[updates] {}` has the unknown value `{s}`; using `{}`", $field, $ty::default().as_str());
                    $ty::default()
                }))
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
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

    /// The channel with this name; `None` for a name that this version does not know.
    pub fn from_name(s: &str) -> Option<Channel> {
        [Channel::Stable, Channel::Beta].into_iter().find(|c| c.as_str() == s)
    }
}
lenient_enum!(Channel, "channel");

/// When the host checks the feed. Only `on-start` is implemented
/// (`docs/UPDATES.md` §8 step 4); an interval is future work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckPolicy {
    #[default]
    OnStart,
}

impl CheckPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckPolicy::OnStart => "on-start",
        }
    }

    /// The policy with this name; `None` for a name that this version does not know.
    pub fn from_name(s: &str) -> Option<CheckPolicy> {
        [CheckPolicy::OnStart].into_iter().find(|c| c.as_str() == s)
    }
}
lenient_enum!(CheckPolicy, "check");

/// When a downloaded update takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
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

impl ApplyPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            ApplyPolicy::NextStart => "next-start",
            ApplyPolicy::Prompt => "prompt",
            ApplyPolicy::Live => "live",
        }
    }

    /// The policy with this name; `None` for a name that this version does not know.
    pub fn from_name(s: &str) -> Option<ApplyPolicy> {
        [ApplyPolicy::NextStart, ApplyPolicy::Prompt, ApplyPolicy::Live].into_iter().find(|c| c.as_str() == s)
    }
}
lenient_enum!(ApplyPolicy, "apply");

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

/// A `[[capabilities]]` table. Lenient, for the manifest: it ignores a
/// field that it does not know. `ProjectConfig::parse` keeps `plinth.toml`
/// strict with `check_strict`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Capability {
    pub name: String,
    /// `ProjectConfig::validate` requires it in `plinth.toml`.
    #[serde(default)]
    pub rationale: String,
}

impl Capability {
    /// The fields of `[[capabilities]]`, for the strict check of `plinth.toml`.
    const FIELDS: &[&str] = &["name", "rationale"];
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
    /// For `core = "patch"`: the version (`MAJOR.MINOR.PATCH`) of the core
    /// build the app was built with. The host takes only patches of this
    /// minor version. `None` for the other policies and for a manifest
    /// written before this field existed (the host then uses the lowest
    /// core the app needs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_built: Option<String>,
    /// The author's update feed configuration (`docs/UPDATES.md` §8 step
    /// 2), carried from `plinth.toml`'s `[updates]` unchanged. `None`
    /// means no updates: a manifest written before this field existed,
    /// or a `plinth.toml` with no `[updates]` section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdatesConfig>,
    /// The app's theme token values, carried from `plinth.toml`'s `[theme]`
    /// unchanged. Data only: it grants nothing. A host checks it again
    /// (`theme::ThemeOverride::resolve`) and ignores an override that fails.
    /// A `theme` that cannot be read reads as `None` (and is logged), so a
    /// bad theme never makes a package unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_theme")]
    pub theme: Option<theme::ThemeOverride>,
}

/// Reads the manifest's `theme`; a value of the wrong shape gives `None`.
fn lenient_theme<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<theme::ThemeOverride>, D::Error> {
    let raw = Option::<serde_json::Value>::deserialize(d)?;
    Ok(raw.and_then(|v| match serde_json::from_value::<theme::ThemeOverride>(v) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("manifest.toml: `[theme]` is not valid ({e}); the host theme is used");
            None
        }
    }))
}

fn default_core_field() -> String {
    CorePolicy::Follow.as_str().to_owned()
}

impl ProjectConfig {
    pub fn parse(text: &str) -> Result<Self> {
        let table: toml::Table = toml::from_str(text).context("plinth.toml is not valid")?;
        // The theme first: its messages name the key that is wrong.
        if let Some(t) = table.get("theme") {
            theme::ThemeOverride::from_toml(t).map_err(|e| anyhow::anyhow!("plinth.toml: {e}"))?;
        }
        let c: ProjectConfig = toml::from_str(text).context("plinth.toml is not valid")?;
        Self::check_strict(&table)?;
        c.validate()?;
        Ok(c)
    }

    /// `plinth.toml` is strict (`docs/COMPATIBILITY.md` §4): an unknown
    /// field or an unknown enum value is most likely a typing error. The
    /// shared types `Capability` and `UpdatesConfig` are lenient for the
    /// manifest, so this checks the nested tables here. (`ProjectConfig`
    /// itself denies unknown top-level fields, and `CorePolicy` is strict.)
    fn check_strict(table: &toml::Table) -> Result<()> {
        fn known_fields(t: &toml::Table, fields: &[&str], section: &str) -> Result<()> {
            if let Some(k) = t.keys().find(|k| !fields.contains(&k.as_str())) {
                bail!("plinth.toml: `{section}` has an unknown field `{k}` (known: {})", fields.join(", "));
            }
            Ok(())
        }
        if let Some(caps) = table.get("capabilities").and_then(|v| v.as_array()) {
            for cap in caps.iter().filter_map(|c| c.as_table()) {
                known_fields(cap, Capability::FIELDS, "[[capabilities]]")?;
            }
        }
        if let Some(updates) = table.get("updates").and_then(|v| v.as_table()) {
            known_fields(updates, UpdatesConfig::FIELDS, "[updates]")?;
            let checks: [(&str, fn(&str) -> bool, &str); 3] = [
                ("channel", |s| Channel::from_name(s).is_some(), "\"stable\", \"beta\""),
                ("check", |s| CheckPolicy::from_name(s).is_some(), "\"on-start\""),
                ("apply", |s| ApplyPolicy::from_name(s).is_some(), "\"next-start\", \"prompt\""),
            ];
            for (field, known, values) in checks {
                if let Some(v) = updates.get(field).and_then(|v| v.as_str())
                    && !known(v)
                {
                    bail!("plinth.toml: `[updates] {field}` has the unknown value \"{v}\" (use {values})");
                }
            }
        }
        Ok(())
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

    /// Checks the `[theme]` against the app's named accent: merges it with
    /// the host palette and runs the contrast guard in light and dark. The
    /// accent is known only after the compile, so this is a separate step
    /// from `validate`. `Ok` when there is no `[theme]`.
    pub fn check_theme(&self, named_accent: Option<&str>) -> Result<()> {
        if let Some(t) = &self.theme {
            t.resolve(named_accent).map_err(|e| anyhow::anyhow!("plinth.toml: {e}"))?;
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
            core_built: None,
            theme: self.theme.clone(),
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
/// the host does not use (`source/…`, unknown entries), so that `write` gives them back
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
    /// The entries that the host does not use (`source/…` and entries that
    /// this version does not know). They are kept and signed, but never
    /// served as assets and never run.
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
        // The profile first: a newer format gives `NewerFormat`, not an
        // error from a rule of this format.
        {
            let mut f = zip.by_name("plinth-profile").map_err(|_| anyhow::anyhow!("the package has no plinth-profile"))?;
            let mut raw = Vec::new();
            (&mut f).take(64).read_to_end(&mut raw)?;
            check_profile(&raw)?;
        }
        let mut seen = std::collections::HashSet::new();
        let mut total = 0u64;
        let (mut profile, mut manifest, mut files) = (None, None, Vec::new());
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name().context("the package has an invalid file name")?.into_owned();
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
                // The entry module, `assets/…`, `signature.json`,
                // `source/…` and every entry that this version does not
                // know. An unknown entry goes to `other`: it is kept and
                // signed, never served and never run.
                _ => files.push((name, data)),
            }
        }
        let profile = profile.context("the package has no plinth-profile")?;
        check_profile(&profile)?;
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

    const BASE: &str = "id = \"dev.plinth.test\"
name = \"Test\"
version = \"0.1.0\"
publisher = \"me\"
";

    fn themed(extra: &str) -> Result<ProjectConfig> {
        ProjectConfig::parse(&format!("{BASE}{extra}"))
    }

    #[test]
    fn plinth_toml_theme_parses_and_reports_errors() {
        let c = themed("[theme]
background = \"#101820\"
[theme.accent]
light = \"#0a5cd6\"
dark = \"#6ea8ff\"
").unwrap();
        assert_eq!(c.theme.as_ref().unwrap().0.len(), 2);
        assert!(config().theme.is_none());
        let msg = |extra: &str| format!("{:#}", themed(extra).unwrap_err());
        assert!(msg("[theme]
brand = \"#112233\"
").contains("unknown key `brand`"));
        assert!(msg("[theme]
text = \"blue\"
").contains("hex color"));
        assert!(msg("[theme]
text = 3
").contains("hex string or a table"));
        let many: String = (0..13).map(|i| format!("k{i} = \"#000000\"
")).collect();
        assert!(msg(&format!("[theme]
{many}")).contains("13 keys"));
    }

    #[test]
    fn check_theme_runs_the_contrast_guard() {
        let ok = themed("[theme]
[theme.text]
light = \"#111111\"
dark = \"#f0f0f0\"
[theme.background]
light = \"#fafafa\"
dark = \"#101010\"
").unwrap();
        ok.check_theme(Some("blue")).unwrap();
        let bad = themed("[theme]
text = \"#dddddd\"
background = \"#fafafa\"
").unwrap();
        let e = format!("{:#}", bad.check_theme(None).unwrap_err());
        assert!(e.contains("`text` on `background`") && e.contains("4.5:1"), "{e}");
        config().check_theme(None).unwrap();
    }

    /// An app with no `[theme]` gets the same manifest bytes as before.
    #[test]
    fn no_theme_leaves_the_manifest_bytes_unchanged() {
        let wasm = b" asm x".to_vec();
        let m = config().manifest("1.0", "plinth-rt/test", Some("teal".into()), &wasm);
        let text = manifest_toml(&m).unwrap();
        assert!(!text.contains("theme"), "{text}");
    }

    #[test]
    fn manifest_theme_round_trips_through_a_package() {
        let wasm = b" asm x".to_vec();
        let c = themed("[theme]
background = \"#101820\"
[theme.accent]
light = \"#0a5cd6\"
dark = \"#6ea8ff\"
").unwrap();
        let pkg = Package::new(c.manifest("1.0", "plinth-rt/test", None, &wasm), wasm, vec![]);
        let back = Package::read(&pkg.write().unwrap()).unwrap();
        assert_eq!(back.manifest.theme, c.theme);
        let text = manifest_toml(&back.manifest).unwrap();
        assert!(text.contains("[theme]") && text.contains("[theme.accent]"), "{text}");
    }

    /// A package with a theme that cannot be read still opens; the host
    /// theme is used (never crash on package data).
    #[test]
    fn unreadable_manifest_theme_reads_as_none() {
        for theme in ["theme = 5
", "[theme]
text = 5
", "[theme]
text = { light = \"#000000\" }
"] {
            let wasm = b" asm x".to_vec();
            let mut pkg = Package::new(config().manifest("1.0", "plinth-rt/test", None, &wasm), wasm, vec![]);
            let good = pkg.write().unwrap();
            // Rewrite manifest.toml with the bad theme.
            let m = String::from_utf8(pkg.manifest_bytes().unwrap().into_owned()).unwrap();
            let bad = format!("{m}{theme}");
            let mut zip_in = zip::ZipArchive::new(Cursor::new(good)).unwrap();
            let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let opts = zip::write::SimpleFileOptions::default();
            for i in 0..zip_in.len() {
                let mut f = zip_in.by_index(i).unwrap();
                let name = f.name().unwrap().into_owned();
                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();
                out.start_file(name.as_str(), opts).unwrap();
                out.write_all(if name == "manifest.toml" { bad.as_bytes() } else { &data }).unwrap();
            }
            let bytes = out.finish().unwrap().into_inner();
            let read = Package::read(&bytes).unwrap_or_else(|e| panic!("{theme}: {e:#}"));
            assert!(read.manifest.theme.is_none(), "{theme}");
            pkg.manifest.theme = None;
        }
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

    // -- Format compatibility (docs/COMPATIBILITY.md §4) --------------------

    /// A zip with these entries, in order.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A manifest from a newer author tool: unknown fields at every level
    /// and unknown enum values in `[updates]`.
    fn newer_manifest(digest: &str) -> String {
        format!(
            r#"id = "dev.plinth.test"
name = "Test"
version = "0.2.0"
publisher = "me"
ui-api = "1.0"
runtime = "plinth-rt/test"
entry = "app.wasm"
digest = "{digest}"
core = "nightly"
future-top-level = "x"

[future-table]
a = 1

[[capabilities]]
name = "store.kv"
rationale = "Save."
future-field = true

[updates]
feed = "https://example.com/feed.json"
channel = "canary"
check = "hourly"
apply = "instant"
future-field = [1, 2]
"#
        )
    }

    #[test]
    fn a_manifest_with_unknown_fields_and_enum_values_reads() {
        let wasm = b"\0asm".to_vec();
        let manifest = newer_manifest(&hex(&Sha256::digest(&wasm)));
        let bytes = zip_of(&[("plinth-profile", b"plinth/1"), ("manifest.toml", manifest.as_bytes()), ("app.wasm", &wasm)]);
        let pkg = Package::read(&bytes).unwrap();
        assert_eq!(pkg.manifest.capabilities, vec![Capability { name: "store.kv".into(), rationale: "Save.".into() }]);
        let updates = pkg.manifest.updates.as_ref().unwrap();
        assert_eq!(updates.feed, "https://example.com/feed.json");
        // An unknown enum value maps to the default of the field.
        assert_eq!((updates.channel, updates.check, updates.apply), (Channel::Stable, CheckPolicy::OnStart, ApplyPolicy::NextStart));
        assert_eq!(CorePolicy::parse(&pkg.manifest.core), CorePolicy::Follow);
        // The raw bytes stay: the unknown fields are written back unchanged.
        assert_eq!(&*pkg.manifest_bytes().unwrap(), manifest.as_bytes());
        let again = Package::read(&pkg.write().unwrap()).unwrap();
        assert_eq!(&*again.manifest_bytes().unwrap(), manifest.as_bytes());
    }

    #[test]
    fn plinth_toml_stays_strict() {
        let base = "id = \"dev.plinth.test\"\nname = \"Test\"\nversion = \"0.1.0\"\npublisher = \"me\"\n";
        assert!(ProjectConfig::parse(base).is_ok());
        let good_updates = "[updates]\nfeed = \"https://example.com/feed.json\"\nchannel = \"beta\"\ncheck = \"on-start\"\napply = \"prompt\"\n";
        assert!(ProjectConfig::parse(&format!("{base}{good_updates}")).is_ok());
        let bad = [
            ("future-top-level = \"x\"\n", "future-top-level"),
            ("[future-table]\na = 1\n", "future-table"),
            ("[[capabilities]]\nname = \"store.kv\"\nrationale = \"Save.\"\nfuture-field = true\n", "future-field"),
            ("[[capabilities]]\nname = \"store.kv\"\nrationale = \"Save.\"\nrationnale = \"typo\"\n", "rationnale"),
            ("[updates]\nfeed = \"https://example.com/feed.json\"\nfuture-field = 1\n", "future-field"),
            ("[updates]\nfeed = \"https://example.com/feed.json\"\nchannel = \"canary\"\n", "canary"),
            ("[updates]\nfeed = \"https://example.com/feed.json\"\ncheck = \"hourly\"\n", "hourly"),
            ("[updates]\nfeed = \"https://example.com/feed.json\"\napply = \"instant\"\n", "instant"),
            ("core = \"nightly\"\n", "nightly"),
        ];
        for (extra, word) in bad {
            let err = ProjectConfig::parse(&format!("{base}{extra}")).unwrap_err();
            assert!(format!("{err:#}").contains(word), "{extra}: {err:#}");
        }
    }

    #[test]
    fn an_unknown_entry_is_kept_but_never_an_asset() {
        let wasm = b"\0asm".to_vec();
        let pkg = Package::new(config().manifest("1.0", "plinth-rt/test", None, &wasm), wasm.clone(), vec![("assets/a.txt".into(), b"hi".to_vec())]);
        let manifest = manifest_toml(&pkg.manifest).unwrap();
        let bytes = zip_of(&[
            ("plinth-profile", b"plinth/1"),
            ("future.bin", b"new data"),
            ("manifest.toml", manifest.as_bytes()),
            ("app.wasm", &wasm),
            ("assets/a.txt", b"hi"),
            ("future/dir/x.wasm", b"\0asm other code"),
            ("source/app/main.tsx", b"export {}"),
        ]);
        let back = Package::read(&bytes).unwrap();
        assert_eq!(back.assets, vec![("assets/a.txt".to_owned(), b"hi".to_vec())]);
        let names: Vec<&str> = back.other_entries().iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["future.bin", "future/dir/x.wasm", "source/app/main.tsx"]);
        assert_eq!(back.component, wasm);
        // `write` keeps them.
        let again = Package::read(&back.write().unwrap()).unwrap();
        assert_eq!(again.other_entries(), back.other_entries());
    }

    #[test]
    fn a_newer_profile_needs_a_newer_plinth() {
        let wasm = b"\0asm".to_vec();
        let manifest = manifest_toml(&config().manifest("1.0", "plinth-rt/test", None, &wasm)).unwrap();
        let with_profile =
            |p: &[u8]| Package::read(&zip_of(&[("plinth-profile", p), ("manifest.toml", manifest.as_bytes()), ("app.wasm", &wasm)]));
        assert!(with_profile(b"plinth/1").is_ok());
        assert!(with_profile(b"plinth/1\n").is_ok());
        for p in [&b"plinth/2"[..], b"plinth/2\n", b"plinth/17"] {
            let err = with_profile(p).unwrap_err();
            let newer = err.downcast_ref::<NewerFormat>().expect("a NewerFormat error");
            assert!(err.to_string().starts_with("This app needs a newer version of Plinth (package format plinth/"), "{err}");
            assert_eq!(newer.profile, String::from_utf8_lossy(p).trim());
        }
        // A newer format may change everything else: the message is the same.
        let err = Package::read(&zip_of(&[("plinth-profile", b"plinth/2"), ("other.toml", b"x = 1")])).unwrap_err();
        assert!(err.is::<NewerFormat>(), "{err:#}");
        for p in [&b"plinth/0"[..], b"plinth/", b"plinth/x", b"plinth/01", b"plinth/-1", b"garbage", b"", b"\xff\xfe"] {
            let err = with_profile(p).unwrap_err();
            assert!(!err.is::<NewerFormat>(), "{:?}", String::from_utf8_lossy(p));
            assert!(err.to_string().contains("not valid"), "{err}");
        }
        assert!(!Package::read(&zip_of(&[("manifest.toml", manifest.as_bytes())])).unwrap_err().is::<NewerFormat>());
    }
}
