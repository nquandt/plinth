//! The static-registry generator (`docs/REGISTRY.md` §8).
//!
//! `build` reads the `.plnt` files in a folder (and its `incoming/`
//! subfolder), validates each, and writes/updates the registry files. It is
//! idempotent: an existing version is never changed, and every generated
//! field is derived from the stored data (not from the wall clock), so
//! running it again on the same input produces byte-identical output.

use crate::{AppDocument, AppList, AppSummary, CapabilityEntry, CoreEntry, CoresIndex, Resources, SCHEMA, ServiceIndex, VersionEntry, hex_digest, sort_versions, to_stable_json};
use anyhow::{Context as _, Result, bail};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Copies the built-in core into `cores/` and writes `cores/index.json`.
    pub with_core: bool,
    /// The publisher key ids (`ed25519:…`) that the web App Hub trusts as
    /// Hub keys (`docs/web-hub.md`, `docs/HUB.md` §4.1). When it is not
    /// empty, the build writes `hub.json` at the registry root; the page
    /// gives `hub.manage` only to a Hub package that one of them signed.
    pub hub_trusted_keys: Vec<String>,
}

/// The app id of the Hub app (`examples/hub`) that the web App Hub runs.
pub const HUB_APP_ID: &str = "dev.plinth.hub";

/// `hub.json`: the static configuration of the web App Hub.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct WebHubConfig {
    pub schema: String,
    /// The app id of the Hub app in this registry.
    pub hub: String,
    #[serde(rename = "trustedKeys")]
    pub trusted_keys: Vec<String>,
}

/// Builds (or updates) the static registry at `folder`.
pub fn build(folder: &Path, options: &Options) -> Result<Report> {
    std::fs::create_dir_all(folder).with_context(|| format!("create {}", folder.display()))?;
    let mut report = Report::default();

    for path in incoming_packages(folder)? {
        let bytes = std::fs::read(&path).with_context(|| format!("read {}", path.display()))?;
        add_package(folder, &bytes, &mut report).with_context(|| format!("{}", path.display()))?;
    }

    if options.with_core {
        add_core(folder, &mut report)?;
    }

    regenerate_indexes(folder)?;
    if !options.hub_trusted_keys.is_empty() {
        let config = WebHubConfig { schema: "plinth.web-hub/1".into(), hub: HUB_APP_ID.into(), trusted_keys: options.hub_trusted_keys.clone() };
        write_atomic(&folder.join("hub.json"), to_stable_json(&config)?.as_bytes())?;
    }
    Ok(report)
}

#[derive(Debug, Default)]
pub struct Report {
    pub added: Vec<(String, String)>,
    pub unchanged: Vec<(String, String)>,
}

/// `.plnt` files directly in `folder`, then in `folder/incoming/`, each list
/// sorted by file name for a deterministic processing order.
fn incoming_packages(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for dir in [folder.to_path_buf(), folder.join("incoming")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(plinth_package::EXTENSION))
            .collect();
        found.sort();
        out.extend(found);
    }
    Ok(out)
}

/// Validates one package the same way `plinth validate` does, then stores
/// it and adds its version to the app document (`docs/REGISTRY.md` §8
/// steps 1-3).
fn add_package(folder: &Path, bytes: &[u8], report: &mut Report) -> Result<()> {
    let pkg = plinth_package::Package::read(bytes).context("not a valid .plnt package")?;
    plinth_link::cores::link_app(&pkg.component).context("the package does not link against an installed core")?;
    // A signature that does not check out is refused; an unsigned package
    // is still allowed (`docs/HUB.md` §6.1, draft-1 registries keep
    // working with no signatures).
    let signer = plinth_package::signature::verify(&pkg).context("the package signature does not check out")?;
    let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
    plinth_link::split::check_capabilities(&pkg.component, &declared)?;
    let reachable = plinth_link::split::reachable_capabilities(&pkg.component)?;

    let digest = hex_digest(bytes);
    let packages_dir = folder.join("packages");
    std::fs::create_dir_all(&packages_dir)?;
    let package_path = packages_dir.join(format!("{digest}.{}", plinth_package::EXTENSION));
    if !package_path.exists() {
        write_atomic(&package_path, bytes)?;
    }

    let app_dir = folder.join("apps").join(&pkg.manifest.id);
    std::fs::create_dir_all(&app_dir)?;
    let doc_path = app_dir.join("index.json");
    let mut doc = read_app_document(&doc_path)?.unwrap_or_else(|| AppDocument {
        schema: SCHEMA.into(),
        id: pkg.manifest.id.clone(),
        name: pkg.manifest.name.clone(),
        summary: None,
        description: None,
        publisher: pkg.manifest.publisher.clone(),
        homepage: None,
        icon: pkg.manifest.icon.clone(),
        screenshots: Vec::new(),
        versions: Vec::new(),
    });
    doc.name = pkg.manifest.name.clone();
    doc.publisher = pkg.manifest.publisher.clone();
    // The icon (`docs/REGISTRY.md` §3, §8): the package asset that the
    // manifest names, copied next to the app document at the same relative
    // path, so `icon` resolves both in the package and in the registry.
    if let Some(icon) = pkg.manifest.icon.as_deref().map(|i| i.trim_start_matches("./"))
        && let Some((_, bytes)) = pkg.assets.iter().find(|(path, _)| path == icon)
        && !icon.split('/').any(|part| part.is_empty() || part == "." || part == "..")
    {
        write_atomic(&app_dir.join(icon), bytes)?;
        doc.icon = Some(icon.to_owned());
    }

    if let Some(existing) = doc.versions.iter().find(|v| v.version == pkg.manifest.version) {
        if existing.sha256 != digest {
            bail!(
                "a different package is already registered as {}@{} (sha256 {} on disk, {} given)",
                pkg.manifest.id,
                pkg.manifest.version,
                existing.sha256,
                digest
            );
        }
        report.unchanged.push((pkg.manifest.id.clone(), pkg.manifest.version.clone()));
        return Ok(());
    }

    // A new version must be signed by the same key as the previous one
    // (`docs/HUB.md` §6.1); an unsigned app, or a first version, imposes no
    // such rule. Key rotation statements are future work.
    if let Some(previous) = doc.versions.first()
        && let (Some(old_key), Some(new)) = (&previous.signer, &signer)
        && old_key != &new.key
    {
        bail!(
            "{} is signed by a different key ({}) than the previous version ({}); key rotation is not supported yet",
            pkg.manifest.id,
            new.key,
            old_key
        );
    }

    let core = plinth_link::split::app_core_version(&pkg.component).map(|(maj, min)| format!("{maj}.{min}")).unwrap_or_default();
    let entry = VersionEntry {
        version: pkg.manifest.version.clone(),
        sha256: digest,
        size: bytes.len() as u64,
        core,
        ui_api: pkg.manifest.ui_api.clone(),
        capabilities: pkg.manifest.capabilities.iter().map(|c| CapabilityEntry { name: c.name.clone(), rationale: c.rationale.clone() }).collect(),
        reachable: reachable.into_iter().collect(),
        published: now_rfc3339(),
        yanked: None,
        signer: signer.map(|s| s.key),
    };
    report.added.push((pkg.manifest.id.clone(), entry.version.clone()));
    doc.versions.push(entry);
    sort_versions(&mut doc.versions);
    write_atomic(&doc_path, to_stable_json(&doc)?.as_bytes())?;
    Ok(())
}

fn read_app_document(path: &Path) -> Result<Option<AppDocument>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?))
}

/// Copies the built-in core to `cores/<version>/core.wasm`
/// (`docs/REGISTRY.md` §8 step 5).
fn add_core(folder: &Path, _report: &mut Report) -> Result<()> {
    let core = plinth_link::link::runtime();
    let (major, minor) = plinth_link::cores::core_version(core).context("the built-in core has no version")?;
    let version = format!("{major}.{minor}");
    let dir = folder.join("cores").join(&version);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("core.wasm");
    if !path.exists() {
        write_atomic(&path, core)?;
    }
    Ok(())
}

/// Rewrites `apps/index.json` and `plinth-registry.json` from the app
/// documents and the cores directory on disk (`docs/REGISTRY.md` §8 step
/// 4), so the output does not depend on which packages were added in this
/// run or in what order.
fn regenerate_indexes(folder: &Path) -> Result<()> {
    let apps_dir = folder.join("apps");
    let mut ids: Vec<String> = std::fs::read_dir(&apps_dir)
        .map(|rd| rd.flatten().filter(|e| e.path().join("index.json").exists()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    ids.sort();

    let mut summaries = Vec::new();
    for id in &ids {
        let doc: AppDocument = serde_json::from_str(&std::fs::read_to_string(apps_dir.join(id).join("index.json"))?)?;
        if let Some(latest) = doc.latest() {
            summaries.push(AppSummary {
                id: doc.id.clone(),
                name: doc.name.clone(),
                summary: doc.summary.clone(),
                publisher: doc.publisher.clone(),
                latest: latest.version.clone(),
                categories: Vec::new(),
                // Relative to the app list's own URL, `apps/index.json`
                // (`docs/REGISTRY.md` §3).
                icon: doc.icon.clone().map(|i| format!("{}/{}", doc.id, i)),
                capabilities: latest.capabilities.iter().map(|c| c.name.clone()).collect(),
                labels: latest.capabilities.iter().map(|c| crate::capability_label(&c.name, &c.rationale)).collect(),
                updated: latest.published.clone(),
            });
        }
    }
    let app_list = AppList { schema: SCHEMA.into(), apps: summaries, next: None };
    write_atomic(&folder.join("apps").join("index.json"), to_stable_json(&app_list)?.as_bytes())?;

    let cores_dir = folder.join("cores");
    let mut core_versions: Vec<String> = std::fs::read_dir(&cores_dir)
        .map(|rd| rd.flatten().filter(|e| e.path().join("core.wasm").exists()).filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    core_versions.sort();
    let mut cores = Vec::new();
    for v in &core_versions {
        let bytes = std::fs::read(cores_dir.join(v).join("core.wasm"))?;
        cores.push(CoreEntry { version: v.clone(), sha256: hex_digest(&bytes), size: bytes.len() as u64 });
    }
    if !cores.is_empty() {
        let idx = CoresIndex { schema: SCHEMA.into(), cores };
        write_atomic(&cores_dir.join("index.json"), to_stable_json(&idx)?.as_bytes())?;
    }

    let generated = app_list.apps.iter().map(|a| a.updated.clone()).max().unwrap_or_else(|| "1970-01-01T00:00:00Z".into());
    let index = ServiceIndex {
        schema: SCHEMA.into(),
        name: None,
        description: None,
        resources: Resources {
            apps: "apps/index.json".into(),
            app: "apps/{id}/index.json".into(),
            package: "packages/{sha256}.plnt".into(),
            cores: if core_versions.is_empty() { None } else { Some("cores/index.json".into()) },
            core: if core_versions.is_empty() { None } else { Some("cores/{version}/core.wasm".into()) },
            search: None,
        },
        generated,
    };
    write_atomic(&folder.join("plinth-registry.json"), to_stable_json(&index)?.as_bytes())?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(existing) = std::fs::read(path)
        && existing == bytes
    {
        return Ok(());
    }
    let parent = path.parent().context("path has no parent")?;
    std::fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".{}.tmp-{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// `now()` as `YYYY-MM-DDTHH:MM:SSZ`, with no `chrono` dependency (Howard
/// Hinnant's civil-from-days algorithm).
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_date_sane() {
        // 2026-10-06 00:00:00Z.
        let secs: u64 = 1_791_244_800;
        let days = (secs / 86400) as i64;
        assert_eq!(civil_from_days(days), (2026, 10, 6));
    }
}
