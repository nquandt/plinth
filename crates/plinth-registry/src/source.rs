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
    /// The request headers, and the origin that gets them.
    access: Access,
}

impl Source {
    /// Opens a registry at `base` by reading its service index.
    pub fn open(base: &str) -> Result<Source> {
        Self::open_with(base, RequestHeaders::default())
    }

    /// `open` with request headers (`docs/HUB.md` §5.4). The client sends
    /// them only to the origin of `base`: a redirect to another host does
    /// not get them.
    pub fn open_with(base: &str, headers: RequestHeaders) -> Result<Source> {
        let base_loc = Loc::parse_base(base)?;
        let origin = match &base_loc {
            Loc::Url(u) => origin_of(u),
            Loc::Path(_) => None,
        };
        let access = Access { headers, origin };
        let index_loc = base_loc.join("plinth-registry.json")?;
        let bytes = fetch(&index_loc, &access)?;
        let index: ServiceIndex = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", index_loc.display()))?;
        check_schema(&index.schema)?;
        Ok(Source { base: base_loc, index, index_loc, access })
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
            let bytes = fetch(&loc, &self.access)?;
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
        let bytes = fetch(&loc, &self.access)?;
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
        let bytes = fetch_limited(&loc, MAX_PACKAGE, &self.access)?;
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
            let bytes = fetch(&loc, &self.access)?;
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
        let bytes = fetch(&loc, &self.access)?;
        let idx: CoresIndex = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
        check_schema(&idx.schema)?;
        Ok(idx.cores)
    }

    /// Downloads one core by version.
    pub fn core(&self, version: &str) -> Result<Vec<u8>> {
        let template = self.index.resources.core.as_deref().context("this registry serves no cores")?;
        let loc = self.resource(template, &[("version", version)])?;
        fetch_limited(&loc, MAX_PACKAGE, &self.access)
    }

    /// `hub.json` at the root of the registry (`docs/web-hub.md` §5): the Hub
    /// app id and the keys that the registry owner names as Hub keys. It is
    /// read with the request headers of the source. Reading it trusts
    /// nothing: the caller must ask the user before it saves a key.
    pub fn hub_config(&self) -> Result<crate::build::WebHubConfig> {
        let loc = self.index_loc.join("hub.json")?;
        let bytes = fetch(&loc, &self.access).with_context(|| format!("this registry has no usable hub.json ({})", loc.display()))?;
        let config: crate::build::WebHubConfig = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", loc.display()))?;
        if config.schema != "plinth.web-hub/1" {
            bail!("{} has the unknown schema `{}`", loc.display(), config.schema);
        }
        Ok(config)
    }

    /// The base this source was opened with, as given.
    pub fn base_display(&self) -> String {
        self.base.display()
    }
}

/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 5;

/// The error for a redirect to a Cloudflare Access login page.
const ACCESS_MESSAGE: &str = "the registry needs a login or a token (Cloudflare Access); add a service token with `plinth hub source set-token <source>`";

/// Request headers for one source. `Debug` shows the names only.
#[derive(Clone, Default)]
pub struct RequestHeaders(Vec<(String, String)>);

impl RequestHeaders {
    /// Checks each name and value. The error names the header, never the
    /// value.
    pub fn new(headers: Vec<(String, String)>) -> Result<Self> {
        for (name, value) in &headers {
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
                bail!("`{name}` is not a valid header name");
            }
            if value.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f) {
                bail!("the value of the header {name} has a control character");
            }
        }
        Ok(Self(headers))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for RequestHeaders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.0.iter().map(|(n, _)| n)).finish()
    }
}

/// The scheme, host and port of an `http(s)://` URL, in lowercase.
fn origin_of(url: &str) -> Option<(String, String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    let (host, port) = match authority.rfind(':') {
        Some(i) if !authority[i..].contains(']') => (&authority[..i], authority[i + 1..].parse().ok()?),
        _ => (authority, if scheme == "https" { 443 } else { 80 }),
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme, host.to_ascii_lowercase(), port))
}

/// The URL that a `Location` value means, from the URL it answered.
fn redirect_target(from: &str, location: &str) -> String {
    if location.starts_with("http://") || location.starts_with("https://") {
        return location.to_owned();
    }
    let Some((scheme, rest)) = from.split_once("://") else { return location.to_owned() };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if let Some(path) = location.strip_prefix('/') {
        return format!("{scheme}://{authority}/{path}");
    }
    let dir_end = rest[authority_end..].split(['?', '#']).next().and_then(|p| p.rfind('/')).map_or(authority_end, |i| authority_end + i);
    format!("{scheme}://{}/{location}", &rest[..dir_end])
}

/// True for the host of a Cloudflare Access login page.
fn is_access_login(url: &str) -> bool {
    origin_of(url).is_some_and(|(_, host, _)| host == "cloudflareaccess.com" || host.ends_with(".cloudflareaccess.com"))
}

/// What a fetch sends: the headers, and the one origin that gets them.
#[derive(Clone, Default)]
struct Access {
    headers: RequestHeaders,
    origin: Option<(String, String, u16)>,
}

fn fetch(loc: &Loc, access: &Access) -> Result<Vec<u8>> {
    fetch_limited(loc, MAX_JSON, access)
}

/// Reads a URL. It follows the redirects itself, so the headers go only to
/// the origin of the source (same scheme, host and port): a redirect to
/// another origin does not get them.
fn get_url(start: &str, limit: u64, access: &Access) -> Result<Vec<u8>> {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let mut url = start.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        let mut req = agent.get(&url);
        if !access.headers.is_empty() && origin_of(&url).is_some() && origin_of(&url) == access.origin {
            for (name, value) in &access.headers.0 {
                req = req.set(name, value);
            }
        }
        let resp = req.call().with_context(|| format!("fetch {url}"))?;
        if matches!(resp.status(), 301 | 302 | 303 | 307 | 308) {
            let location = resp.header("location").with_context(|| format!("fetch {url}: a redirect with no Location"))?;
            let next = redirect_target(&url, location);
            if is_access_login(&next) {
                bail!("{ACCESS_MESSAGE}");
            }
            // The same rule as a source base: `http://` only for localhost.
            match Loc::parse_base(&next) {
                Ok(Loc::Url(next)) => url = next,
                _ => bail!("fetch {url}: a redirect to a place that this client does not read"),
            }
            continue;
        }
        let mut buf = Vec::new();
        resp.into_reader().take(limit + 1).read_to_end(&mut buf)?;
        if buf.len() as u64 > limit {
            bail!("{url} is larger than the {} MiB limit", limit >> 20);
        }
        return Ok(buf);
    }
    bail!("fetch {start}: too many redirects")
}

fn fetch_limited(loc: &Loc, limit: u64, access: &Access) -> Result<Vec<u8>> {
    match loc {
        Loc::Path(p) => {
            let meta = std::fs::metadata(p).with_context(|| format!("read {}", p.display()))?;
            if meta.len() > limit {
                bail!("{} is larger than the {} MiB limit", p.display(), limit >> 20);
            }
            std::fs::read(p).with_context(|| format!("read {}", p.display()))
        }
        Loc::Url(u) => get_url(u, limit, access),
    }
}
