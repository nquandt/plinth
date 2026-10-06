//! A client for one registry (`docs/REGISTRY.md` §2, §9).
//!
//! The base is an `https://` URL, an `http://` URL (only `localhost` or
//! `127.0.0.1`), a `file:` URL, or a plain folder path. Resource templates
//! are resolved relative to the base (or the app document's own URL, for the
//! URLs inside an app document).

use crate::{AppDocument, AppList, AppSummary, CoresIndex, ServiceIndex, check_schema};
use anyhow::{Context as _, Result, bail};
use std::path::PathBuf;

/// A client limit: no single JSON document or package is read past this.
const MAX_JSON: u64 = 8 << 20;
const MAX_PACKAGE: u64 = 256 << 20;

/// A resolved location: either a URL (http/https) or a local path (from a
/// `file:` URL or a plain folder path).
#[derive(Debug, Clone)]
enum Loc {
    Url(String),
    Path(PathBuf),
}

impl Loc {
    /// Parses a base (the registry's base URL/path, or a template value
    /// that turned out to be absolute).
    fn parse_base(base: &str) -> Result<Loc> {
        if let Some(rest) = base.strip_prefix("file://") {
            return Ok(Loc::Path(PathBuf::from(rest)));
        }
        if let Some(rest) = base.strip_prefix("https://") {
            return Ok(Loc::Url(format!("https://{rest}")));
        }
        if let Some(rest) = base.strip_prefix("http://") {
            let host = rest.split(['/', ':']).next().unwrap_or("");
            if host != "localhost" && host != "127.0.0.1" {
                bail!("http:// is allowed only for localhost and 127.0.0.1, not `{host}`");
            }
            return Ok(Loc::Url(format!("http://{rest}")));
        }
        Ok(Loc::Path(PathBuf::from(base)))
    }

    /// Resolves `rel` (a resource template value, with placeholders already
    /// substituted) against `self`. An absolute `rel` (its own scheme) is
    /// returned as-is.
    fn join(&self, rel: &str) -> Result<Loc> {
        if rel.starts_with("https://") || rel.starts_with("http://") || rel.starts_with("file://") {
            return Loc::parse_base(rel);
        }
        match self {
            Loc::Url(base) => {
                // Strip the last path segment of `base` (after its
                // scheme://authority), then append `rel`. The base URLs this
                // crate joins against are always "directory-like" document
                // URLs (the service index, or an app document), so this
                // simple scheme join is enough: it does not need to handle
                // `..`.
                let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
                let dir = match base[scheme_end..].find('/') {
                    Some(i) => {
                        let path_start = scheme_end + i;
                        match base[path_start..].rfind('/') {
                            Some(j) => &base[..path_start + j],
                            None => &base[..path_start],
                        }
                    }
                    None => base.as_str(),
                };
                Ok(Loc::Url(format!("{dir}/{rel}")))
            }
            Loc::Path(base) => {
                let dir = if base.is_dir() { base.clone() } else { base.parent().map(PathBuf::from).unwrap_or_else(|| base.clone()) };
                Ok(Loc::Path(dir.join(rel)))
            }
        }
    }

    fn display(&self) -> String {
        match self {
            Loc::Url(u) => u.clone(),
            Loc::Path(p) => p.display().to_string(),
        }
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Fills `{name}` placeholders in a resource template with percent-encoded
/// values.
fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_owned();
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), &percent_encode(value));
    }
    out
}

/// A client for one registry.
pub struct Source {
    base: Loc,
    index: ServiceIndex,
    /// The service index's own location, for resolving resource templates.
    index_loc: Loc,
}

impl Source {
    /// Opens a registry at `base` by reading its service index.
    pub fn open(base: &str) -> Result<Source> {
        let base_loc = Loc::parse_base(base)?;
        let index_loc = base_loc.join("plinth-registry.json")?;
        let bytes = fetch(&index_loc)?;
        let index: ServiceIndex = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", index_loc.display()))?;
        check_schema(&index.schema)?;
        Ok(Source { base: base_loc, index, index_loc })
    }

    /// The registry's display name, if it set one.
    pub fn name(&self) -> Option<&str> {
        self.index.name.as_deref()
    }

    fn resource(&self, template: &str, vars: &[(&str, &str)]) -> Result<Loc> {
        self.index_loc.join(&fill(template, vars))
    }

    /// Every app summary, following `next` pages.
    pub fn apps(&self) -> Result<Vec<AppSummary>> {
        let mut out = Vec::new();
        let mut loc = self.resource(&self.index.resources.apps, &[])?;
        loop {
            let bytes = fetch(&loc)?;
            let page: AppList = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
            check_schema(&page.schema)?;
            out.extend(page.apps);
            match page.next {
                Some(next) if !next.is_empty() => loc = loc.join(&next)?,
                _ => break,
            }
        }
        Ok(out)
    }

    /// The app document for `id`.
    pub fn app(&self, id: &str) -> Result<AppDocument> {
        let loc = self.resource(&self.index.resources.app, &[("id", id)])?;
        let bytes = fetch(&loc)?;
        let doc: AppDocument = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
        check_schema(&doc.schema)?;
        Ok(doc)
    }

    /// Downloads `id`'s package at `version`, verifies its digest against
    /// the app document, then verifies the manifest id/version
    /// (`docs/REGISTRY.md` §5).
    pub fn package(&self, id: &str, version: &str) -> Result<Vec<u8>> {
        let doc = self.app(id)?;
        let entry = doc.versions.iter().find(|v| v.version == version).with_context(|| format!("{id} has no version {version}"))?;
        let loc = self.resource(&self.index.resources.package, &[("sha256", &entry.sha256)])?;
        let bytes = fetch_limited(&loc, MAX_PACKAGE)?;
        let digest = crate::hex_digest(&bytes);
        if digest != entry.sha256 {
            bail!("the package for {id}@{version} does not match its digest (expected {}, got {digest})", entry.sha256);
        }
        let pkg = plinth_package::Package::read(&bytes).with_context(|| format!("{id}@{version} is not a valid package"))?;
        if pkg.manifest.id != id || pkg.manifest.version != version {
            bail!(
                "the package served for {id}@{version} names {}@{} in its manifest",
                pkg.manifest.id,
                pkg.manifest.version
            );
        }
        Ok(bytes)
    }

    /// Searches the registry: the `search` resource if the registry has
    /// one, else a local filter over `apps()` (`docs/REGISTRY.md` §7).
    pub fn search(&self, q: &str) -> Result<Vec<AppSummary>> {
        if let Some(template) = &self.index.resources.search {
            let loc = self.resource(template, &[("query", q)])?;
            let bytes = fetch(&loc)?;
            let page: AppList = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
            check_schema(&page.schema)?;
            return Ok(page.apps);
        }
        let needle = q.to_lowercase();
        Ok(self
            .apps()?
            .into_iter()
            .filter(|a| {
                a.id.to_lowercase().contains(&needle)
                    || a.name.to_lowercase().contains(&needle)
                    || a.summary.as_deref().unwrap_or_default().to_lowercase().contains(&needle)
                    || a.categories.iter().any(|c| c.to_lowercase().contains(&needle))
            })
            .collect())
    }

    /// The runtime cores the registry serves, if it has any.
    pub fn cores(&self) -> Result<Vec<crate::CoreEntry>> {
        let Some(template) = &self.index.resources.cores else { return Ok(Vec::new()) };
        let loc = self.resource(template, &[])?;
        let bytes = fetch(&loc)?;
        let idx: CoresIndex = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
        check_schema(&idx.schema)?;
        Ok(idx.cores)
    }

    /// Downloads one core by version.
    pub fn core(&self, version: &str) -> Result<Vec<u8>> {
        let template = self.index.resources.core.as_deref().context("this registry serves no cores")?;
        let loc = self.resource(template, &[("version", version)])?;
        fetch_limited(&loc, MAX_PACKAGE)
    }

    /// The base this source was opened with, as given.
    pub fn base_display(&self) -> String {
        self.base.display()
    }
}

fn fetch(loc: &Loc) -> Result<Vec<u8>> {
    fetch_limited(loc, MAX_JSON)
}

fn fetch_limited(loc: &Loc, limit: u64) -> Result<Vec<u8>> {
    match loc {
        Loc::Path(p) => {
            let meta = std::fs::metadata(p).with_context(|| format!("read {}", p.display()))?;
            if meta.len() > limit {
                bail!("{} is larger than the {} MiB limit", p.display(), limit >> 20);
            }
            std::fs::read(p).with_context(|| format!("read {}", p.display()))
        }
        Loc::Url(u) => {
            let resp = ureq::get(u).call().with_context(|| format!("fetch {u}"))?;
            let mut buf = Vec::new();
            use std::io::Read;
            resp.into_reader().take(limit + 1).read_to_end(&mut buf)?;
            if buf.len() as u64 > limit {
                bail!("{u} is larger than the {} MiB limit", limit >> 20);
            }
            Ok(buf)
        }
    }
}
