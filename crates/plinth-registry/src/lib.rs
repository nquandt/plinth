//! The Plinth registry format (`docs/REGISTRY.md`): the document types, a
//! `Source` client that reads a registry (static or dynamic, over HTTP or a
//! local folder), and the static-registry generator (`build`).

pub mod build;
pub mod serve;
pub mod source;

use serde::{Deserialize, Serialize};

/// `plinth.registry/1`: the schema this crate reads and writes.
pub const SCHEMA: &str = "plinth.registry/1";

fn schema_major(s: &str) -> Option<&str> {
    s.strip_prefix("plinth.registry/")
}

/// Refuses a schema whose major version this crate does not know.
pub fn check_schema(s: &str) -> anyhow::Result<()> {
    let ours = schema_major(SCHEMA).unwrap();
    match schema_major(s) {
        Some(major) if major == ours => Ok(()),
        _ => anyhow::bail!("this client does not know the registry schema `{s}`"),
    }
}

// -- The service index (`docs/REGISTRY.md` §2) -------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Resources {
    #[serde(default)]
    pub apps: String,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cores: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceIndex {
    pub schema: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub resources: Resources,
    pub generated: String,
}

// -- The app list (`docs/REGISTRY.md` §3) -------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSummary {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub publisher: String,
    pub latest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppList {
    pub schema: String,
    pub apps: Vec<AppSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

// -- The app document (`docs/REGISTRY.md` §4) ---------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilityEntry {
    pub name: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct VersionEntry {
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub core: String,
    pub ui_api: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reachable: Vec<String>,
    pub published: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yanked: Option<String>,
    /// The signer's key id (`docs/HUB.md` §6.1, phase H1), copied from the
    /// package's `signature.json`. `None` for an unsigned package (draft-1
    /// registries without signatures still work).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDocument {
    pub schema: String,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub publisher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<String>,
    /// Newest to oldest (semver order); `sort_versions` keeps this true.
    pub versions: Vec<VersionEntry>,
}

impl AppDocument {
    /// The newest non-yanked version, if any (ties with yanked allowed when
    /// none is unyanked, so a client still has something to show).
    pub fn latest(&self) -> Option<&VersionEntry> {
        self.versions.iter().find(|v| v.yanked.is_none()).or_else(|| self.versions.first())
    }
}

/// Sorts `versions` from newest to oldest by semver.
pub fn sort_versions(versions: &mut [VersionEntry]) {
    versions.sort_by(|a, b| semver_key(&b.version).cmp(&semver_key(&a.version)));
}

/// `MAJOR.MINOR.PATCH` as a comparable tuple; a version that does not parse
/// sorts as `(0, 0, 0)`.
fn semver_key(v: &str) -> (u64, u64, u64) {
    let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// True if `a` is a newer semver than `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    semver_key(a) > semver_key(b)
}

// -- Cores (`docs/REGISTRY.md` §6) --------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoreEntry {
    pub version: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoresIndex {
    pub schema: String,
    pub cores: Vec<CoreEntry>,
}

// -- Stable JSON output --------------------------------------------------------

/// 2-space pretty JSON with a trailing newline (`docs/REGISTRY.md` §8:
/// "stable order, stable JSON").
pub fn to_stable_json<T: Serialize>(value: &T) -> anyhow::Result<String> {
    let mut s = serde_json::to_string_pretty(value)?;
    s.push('\n');
    Ok(s)
}

pub fn hex_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_newest_first() {
        let mut v = vec![
            VersionEntry {
                version: "1.0.0".into(),
                sha256: "a".into(),
                size: 1,
                core: "1.0".into(),
                ui_api: "1.0".into(),
                capabilities: vec![],
                reachable: vec![],
                published: "t".into(),
                yanked: None,
                signer: None,
            },
            VersionEntry {
                version: "1.2.0".into(),
                sha256: "b".into(),
                size: 1,
                core: "1.0".into(),
                ui_api: "1.0".into(),
                capabilities: vec![],
                reachable: vec![],
                published: "t".into(),
                yanked: None,
                signer: None,
            },
            VersionEntry {
                version: "1.1.5".into(),
                sha256: "c".into(),
                size: 1,
                core: "1.0".into(),
                ui_api: "1.0".into(),
                capabilities: vec![],
                reachable: vec![],
                published: "t".into(),
                yanked: None,
                signer: None,
            },
        ];
        sort_versions(&mut v);
        let order: Vec<&str> = v.iter().map(|e| e.version.as_str()).collect();
        assert_eq!(order, vec!["1.2.0", "1.1.5", "1.0.0"]);
    }

    #[test]
    fn schema_check() {
        assert!(check_schema("plinth.registry/1").is_ok());
        assert!(check_schema("plinth.registry/2").is_err());
        assert!(check_schema("other/1").is_err());
    }

    #[test]
    fn stable_json_has_trailing_newline() {
        let idx = AppList { schema: SCHEMA.into(), apps: vec![], next: None };
        let s = to_stable_json(&idx).unwrap();
        assert!(s.ends_with('\n'));
        assert!(s.starts_with('{'));
    }
}

// -- The Hub's source client (`docs/HUB.md` §5, phase H3 step 2) ------------

/// Reads registry sources for the Hub UI's `search` and `install`
/// (`plinth_hub::SourceClient`). Each call opens the source again, so a
/// changed registry is seen at once.
pub struct HubSources;

impl plinth_hub::SourceClient for HubSources {
    fn search(&self, base: &str, query: &str) -> anyhow::Result<Vec<plinth_hub::SearchHit>> {
        let source = source::Source::open(base)?;
        Ok(source
            .search(query)?
            .into_iter()
            .map(|a| plinth_hub::SearchHit { id: a.id, name: a.name, version: a.latest, description: a.summary.unwrap_or_default() })
            .collect())
    }

    fn latest_package(&self, base: &str, id: &str) -> anyhow::Result<Option<Vec<u8>>> {
        let source = source::Source::open(base)?;
        if !source.apps()?.iter().any(|a| a.id == id) {
            return Ok(None);
        }
        let doc = source.app(id)?;
        let Some(latest) = doc.latest() else { return Ok(None) };
        Ok(Some(source.package(id, &latest.version)?))
    }

    fn latest_version(&self, base: &str, id: &str) -> anyhow::Result<Option<plinth_hub::SourceVersion>> {
        let source = source::Source::open(base)?;
        if !source.apps()?.iter().any(|a| a.id == id) {
            return Ok(None);
        }
        let doc = source.app(id)?;
        Ok(doc.latest().map(|v| plinth_hub::SourceVersion {
            version: v.version.clone(),
            capabilities: v.capabilities.iter().map(|c| c.name.clone()).collect(),
        }))
    }

    fn package(&self, base: &str, id: &str, version: &str) -> anyhow::Result<Vec<u8>> {
        source::Source::open(base)?.package(id, version)
    }
}
