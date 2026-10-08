//! Installed runtime cores (SPEC.md §10.5).
//!
//! A core is a build of `plinth-rt`: one Wasm module, the same file on every
//! host platform. Its custom section `plinth-core` holds its version,
//! `MAJOR.MINOR.PATCH`. An app names only `MAJOR.MINOR` in its own
//! `plinth-core` section: the lowest core it needs. A missing patch part
//! (an old core, or an app's own section) reads as patch 0. A host keeps
//! more than one core, as a version manager does: the core that this tool
//! was built with (built in) and the cores in the cores directory. For each
//! app it picks the core with the major version that the app needs, then
//! the highest minor version that is not lower than the app's, then the
//! highest patch version (SPEC.md §10.5). A core's identity is its version
//! plus the SHA-256 digest of its bytes: two builds can share a version
//! only when the author's policy is `pinned` and installs both by digest.

use crate::{link, split};
use anyhow::{Context as _, Result, bail};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// The custom section that holds the version of a core.
pub const CORE_SECTION: &str = "plinth-core";

/// A core's own version, `(major, minor, patch)`.
pub type Version = (u32, u32, u32);

/// The version that an app declares it needs, `(major, minor)`: the lowest
/// core that has every function the app imports.
pub type Need = (u32, u32);

/// Reads the version of a core (or an app's declared need) from its
/// `plinth-core` section. Accepts `MAJOR.MINOR` (patch reads as 0) and
/// `MAJOR.MINOR.PATCH`.
pub fn core_version(core: &[u8]) -> Option<Version> {
    for payload in wasmparser::Parser::new(0).parse_all(core) {
        if let Ok(wasmparser::Payload::CustomSection(c)) = payload
            && c.name() == CORE_SECTION
        {
            return parse_version(std::str::from_utf8(c.data()).ok()?);
        }
    }
    None
}

/// Parses `MAJOR.MINOR` or `MAJOR.MINOR.PATCH`. A missing patch is 0.
pub fn parse_version(s: &str) -> Option<Version> {
    let mut parts = s.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = match parts.next() {
        Some(p) => p.parse().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Formats a core's full version, `MAJOR.MINOR.PATCH`.
pub fn format_version(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

/// The SHA-256 digest of a core's bytes, in hex: the other half of a
/// core's identity, next to its version (`plinth core list` shows both).
pub fn digest_hex(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// An installed core.
pub struct Core {
    pub version: Version,
    /// The SHA-256 digest of `bytes`, in hex.
    pub digest: String,
    pub bytes: Vec<u8>,
    /// `None` for the built-in core.
    pub path: Option<PathBuf>,
}

/// The directory of installed cores: `PLINTH_CORES_DIR`, or
/// `%LOCALAPPDATA%\plinth\cores` on Windows and
/// `$XDG_DATA_HOME/plinth/cores` (or `~/.local/share/plinth/cores`) elsewhere.
pub fn cores_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PLINTH_CORES_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("plinth").join("cores")
}

/// The built-in core and the cores in `cores_dir()`. A file that is not a
/// core is skipped. Cores are installed one directory per full version
/// (`cores/<major>.<minor>.<patch>/core.wasm`), but this reads any
/// `*/core.wasm` under the cores directory regardless of its directory
/// name, so an old install (`cores/<major>.<minor>/core.wasm`, from before
/// patch versions) still loads with no migration step.
pub fn installed() -> Vec<Core> {
    let builtin = link::runtime();
    let mut cores = vec![Core {
        version: core_version(builtin).expect("the built-in core has a version"),
        digest: digest_hex(builtin),
        bytes: builtin.to_vec(),
        path: None,
    }];
    if let Ok(entries) = std::fs::read_dir(cores_dir()) {
        for entry in entries.flatten() {
            let path = entry.path().join("core.wasm");
            let Ok(bytes) = std::fs::read(&path) else { continue };
            match core_version(&bytes) {
                Some(version) => {
                    let digest = digest_hex(&bytes);
                    if !cores.iter().any(|c| c.version == version && c.digest == digest) {
                        cores.push(Core { version, digest, bytes, path: Some(path) });
                    }
                }
                None => eprintln!("warning: {} is not a Plinth core; skipped", path.display()),
            }
        }
    }
    cores.sort_by_key(|c| c.version);
    cores
}

/// The best core for an app that needs `need`, under the `follow` policy
/// (the default, and today's only behavior): the same major version, then
/// the highest minor version that is not lower than `need`'s, then the
/// highest patch version.
pub fn select(cores: &[Core], need: Need) -> Option<&Core> {
    cores.iter().filter(|c| c.version.0 == need.0 && c.version.1 >= need.1).max_by_key(|c| (c.version.1, c.version.2))
}

/// The best core under the `patch` policy: only patches of the exact minor
/// version the app was built with (`need.1`), the highest patch.
pub fn select_patch(cores: &[Core], need: Need) -> Option<&Core> {
    cores.iter().filter(|c| c.version.0 == need.0 && c.version.1 == need.1).max_by_key(|c| c.version.2)
}

/// The minor version that `patch` keeps to: the one the app was built
/// with (`core-built`), or the lowest one it needs. A `core-built` of
/// another major version, or lower than the need, is ignored.
pub fn patch_minor(need: Need, built: Option<&str>) -> Need {
    match built.and_then(parse_version) {
        Some((major, minor, _)) if major == need.0 && minor >= need.1 => (major, minor),
        _ => need,
    }
}

/// The core under the `pinned` policy: the exact core build the app was
/// built with, by digest.
pub fn select_pinned<'a>(cores: &'a [Core], digest: &str) -> Option<&'a Core> {
    cores.iter().find(|c| c.digest == digest)
}

/// Installs a core into `cores_dir()` and returns its path. A version is
/// installed one time: the same version must be the same file (two
/// different builds that claim the same version are a maintainer error,
/// since a core's identity is its version; `pinned` apps tell builds apart
/// by digest instead).
pub fn install(bytes: &[u8]) -> Result<PathBuf> {
    let version = core_version(bytes).context("the file is not a Plinth core (no `plinth-core` section)")?;
    link::layout(bytes).context("the file is not a usable Plinth core")?;
    let v = format_version(version);
    let dir = cores_dir().join(&v);
    let path = dir.join("core.wasm");
    if let Ok(existing) = std::fs::read(&path) {
        if existing == bytes {
            return Ok(path);
        }
        bail!("core {v} is installed already with other contents ({})", path.display());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Picks the core that an app module needs from the installed cores, under
/// the `follow` policy, links the app into it, and returns a component
/// that is ready to run.
pub fn link_app(app: &[u8]) -> Result<Vec<u8>> {
    let need = split::app_core_version(app).context("app.wasm does not name its core (no `plinth-core` section)")?;
    let cores = installed();
    let Some(core) = select(&cores, need) else {
        let have: Vec<String> = cores.iter().map(|c| format_version(c.version)).collect();
        bail!(
            "the app needs core {}.{} or a later {}.x; the installed cores are {}. Install one with `plinth core install <core.wasm>`",
            need.0,
            need.1,
            need.0,
            have.join(", ")
        );
    };
    split::link_app(&core.bytes, app)
}

/// Picks the core that an app module needs under `policy` (`"follow"`,
/// `"patch"` or `"pinned"`; an unknown value is treated as `"follow"`,
/// since an old manifest with no `core` field also means `follow`),
/// links the app into it, and returns a component that is ready to run.
/// `pinned_digest` is the digest the app's manifest recorded at build
/// time; it is ignored for the other policies. `built` is the manifest's
/// `core-built` (the core version the app was built with): under `patch`
/// the host takes only patches of that minor version. Without it (an old
/// manifest), `patch` uses the lowest core the app needs.
pub fn link_app_with_policy(app: &[u8], policy: &str, pinned_digest: Option<&str>, built: Option<&str>) -> Result<Vec<u8>> {
    let need = split::app_core_version(app).context("app.wasm does not name its core (no `plinth-core` section)")?;
    let cores = installed();
    let core = match policy {
        "patch" => select_patch(&cores, patch_minor(need, built)),
        "pinned" => {
            let Some(digest) = pinned_digest else {
                bail!("the app's `core = \"pinned\"` policy needs a digest in its manifest, but none is recorded");
            };
            match select_pinned(&cores, digest) {
                Some(core) => Some(core),
                None => bail!(
                    "the app needs the exact core it was built with (digest {digest}, core {}.{}), and it is not installed. Install it with `plinth core install <core.wasm>`",
                    need.0,
                    need.1
                ),
            }
        }
        _ => select(&cores, need),
    };
    let Some(core) = core else {
        let have: Vec<String> = cores.iter().map(|c| format_version(c.version)).collect();
        bail!(
            "the app needs core {}.{} (policy `{policy}`) or, under `follow`, a later {}.x; the installed cores are {}. Install one with `plinth core install <core.wasm>`",
            need.0,
            need.1,
            need.0,
            have.join(", ")
        );
    };
    split::link_app(&core.bytes, app)
}

#[cfg(test)]
mod core_version_tests {
    use super::*;

    fn core(major: u32, minor: u32, patch: u32, digest: &str) -> Core {
        Core { version: (major, minor, patch), digest: digest.to_owned(), bytes: Vec::new(), path: None }
    }

    #[test]
    fn parses_two_and_three_part_versions() {
        assert_eq!(parse_version("1.12"), Some((1, 12, 0)));
        assert_eq!(parse_version("1.12.3"), Some((1, 12, 3)));
        assert_eq!(parse_version(" 1.12.3 "), Some((1, 12, 3)));
        assert_eq!(parse_version("1.12.3.4"), None);
        assert_eq!(parse_version("1"), None);
        assert_eq!(parse_version("x.y"), None);
        assert_eq!(format_version((1, 12, 3)), "1.12.3");
    }

    #[test]
    fn follow_picks_highest_minor_then_highest_patch() {
        let cores = vec![core(1, 0, 0, "a"), core(1, 2, 0, "b"), core(1, 5, 0, "c"), core(1, 5, 2, "d"), core(2, 0, 0, "e")];
        assert_eq!(select(&cores, (1, 1)).map(|c| c.version), Some((1, 5, 2)));
        assert_eq!(select(&cores, (1, 5)).map(|c| c.version), Some((1, 5, 2)));
        assert_eq!(select(&cores, (2, 0)).map(|c| c.version), Some((2, 0, 0)));
        assert!(select(&cores, (1, 6)).is_none());
        assert!(select(&cores, (3, 0)).is_none());
    }

    #[test]
    fn patch_stays_on_the_built_minor() {
        let cores = vec![core(1, 5, 0, "a"), core(1, 5, 3, "b"), core(1, 6, 0, "c")];
        // An app built against 1.5 takes only 1.5.x patches, never 1.6.
        assert_eq!(select_patch(&cores, (1, 5)).map(|c| c.version), Some((1, 5, 3)));
        assert!(select_patch(&cores, (1, 7)).is_none());
    }

    #[test]
    fn patch_keeps_to_the_minor_the_app_was_built_with() {
        // An app that needs 1.0 but was built with 1.5.2 takes 1.5.x only.
        assert_eq!(patch_minor((1, 0), Some("1.5.2")), (1, 5));
        // An old manifest with no `core-built`: the lowest need.
        assert_eq!(patch_minor((1, 0), None), (1, 0));
        // A value of another major, a lower minor, or garbage is ignored.
        assert_eq!(patch_minor((1, 3), Some("2.0.0")), (1, 3));
        assert_eq!(patch_minor((1, 3), Some("1.2.0")), (1, 3));
        assert_eq!(patch_minor((1, 3), Some("x")), (1, 3));
    }

    #[test]
    fn pinned_matches_only_the_exact_digest() {
        let cores = vec![core(1, 5, 0, "aaa"), core(1, 5, 0, "bbb")];
        assert_eq!(select_pinned(&cores, "bbb").map(|c| &c.digest[..]), Some("bbb"));
        assert!(select_pinned(&cores, "ccc").is_none());
    }
}
