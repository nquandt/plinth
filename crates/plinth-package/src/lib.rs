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
use std::io::{Cursor, Read, Write};

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
        Ok(())
    }

    /// The manifest for a built app module. `runtime` is the id of the
    /// runtime build that the app needs (SPEC.md §10.2).
    pub fn manifest(&self, ui_api: &str, runtime: &str, accent: Option<String>, app: &[u8]) -> Manifest {
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
        }
    }
}

fn is_semver(v: &str) -> bool {
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A package in memory.
#[derive(Debug, Clone)]
pub struct Package {
    pub manifest: Manifest,
    pub component: Vec<u8>,
    pub assets: Vec<(String, Vec<u8>)>,
}

impl Package {
    pub fn write(&self) -> Result<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file("plinth-profile", stored)?;
        zip.write_all(PROFILE.as_bytes())?;
        zip.start_file("manifest.toml", opts)?;
        zip.write_all(toml::to_string(&self.manifest)?.as_bytes())?;
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
        let manifest: Manifest = toml::from_str(std::str::from_utf8(&manifest.context("the package has no manifest.toml")?)?)
            .context("manifest.toml is not valid")?;
        let pos = files.iter().position(|(n, _)| *n == manifest.entry).context("the manifest entry is not in the package")?;
        let component = files.remove(pos).1;
        if hex(&Sha256::digest(&component)) != manifest.digest {
            bail!("the digest of `{}` does not match the manifest", manifest.entry);
        }
        let assets = files.into_iter().filter(|(n, _)| n.starts_with("assets/")).collect();
        Ok(Package { manifest, component, assets })
    }
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
        let pkg = Package {
            manifest: config().manifest("1.0", "plinth-rt/test", Some("teal".into()), &wasm),
            component: wasm.clone(),
            assets: vec![("assets/a.txt".into(), b"hi".to_vec())],
        };
        let bytes = pkg.write().unwrap();
        assert!(is_package(&bytes));
        let back = Package::read(&bytes).unwrap();
        assert_eq!(back.manifest, pkg.manifest);
        assert_eq!(back.component, wasm);
        assert_eq!(back.assets.len(), 1);
    }

    #[test]
    fn rejects_bad_paths_and_digests() {
        for p in ["../x", "/abs", "a/../b", "C:/x", "a\\b", "a//b"] {
            assert!(check_path(p).is_err(), "{p}");
        }
        let wasm = b"abc".to_vec();
        let mut pkg = Package { manifest: config().manifest("1.0", "plinth-rt/test", None, &wasm), component: wasm, assets: Vec::new() };
        pkg.component = b"tampered".to_vec();
        assert!(Package::read(&pkg.write().unwrap()).is_err());
    }

    #[test]
    fn validates_config() {
        assert!(ProjectConfig::parse("id = \"x\"\nname = \"T\"\nversion = \"0.1.0\"\npublisher = \"me\"\n").is_err());
        assert!(ProjectConfig::parse("id = \"a.b\"\nname = \"T\"\nversion = \"1\"\npublisher = \"me\"\n").is_err());
        assert!(ProjectConfig::parse("id = \"a.b\"\nname = \"T\"\nversion = \"0.1.0\"\npublisher = \"me\"\nextra = 1\n").is_err());
    }
}
