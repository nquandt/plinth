//! Installed runtime cores (SPEC.md §10.5).
//!
//! A core is a build of `plinth-rt`: one Wasm module, the same file on every
//! host platform. Its custom section `plinth-core` holds its version,
//! `MAJOR.MINOR`. A host keeps more than one core, as a version manager
//! does: the core that this tool was built with (built in) and the cores in
//! the cores directory. For each app it picks the core with the major
//! version that the app needs and the highest minor version that is equal or
//! higher than the app's minor version.

use crate::{link, split};
use anyhow::{Context as _, Result, bail};
use std::path::PathBuf;

/// The custom section that holds the version of a core.
pub const CORE_SECTION: &str = "plinth-core";

/// A core version, `(major, minor)`.
pub type Version = (u32, u32);

/// Reads the version of a core from its `plinth-core` section.
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

/// Parses `MAJOR.MINOR`.
pub fn parse_version(s: &str) -> Option<Version> {
    let (major, minor) = s.trim().split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// An installed core.
pub struct Core {
    pub version: Version,
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
/// core is skipped.
pub fn installed() -> Vec<Core> {
    let builtin = link::runtime();
    let mut cores = vec![Core {
        version: core_version(builtin).expect("the built-in core has a version"),
        bytes: builtin.to_vec(),
        path: None,
    }];
    if let Ok(entries) = std::fs::read_dir(cores_dir()) {
        for entry in entries.flatten() {
            let path = entry.path().join("core.wasm");
            let Ok(bytes) = std::fs::read(&path) else { continue };
            match core_version(&bytes) {
                Some(version) if !cores.iter().any(|c| c.version == version) => {
                    cores.push(Core { version, bytes, path: Some(path) })
                }
                Some(_) => {}
                None => eprintln!("warning: {} is not a Plinth core; skipped", path.display()),
            }
        }
    }
    cores.sort_by_key(|c| c.version);
    cores
}

/// The best core for an app that needs `need`: the same major version and
/// the highest minor version that is not lower than `need`'s.
pub fn select(cores: &[Core], need: Version) -> Option<&Core> {
    cores.iter().filter(|c| c.version.0 == need.0 && c.version.1 >= need.1).max_by_key(|c| c.version.1)
}

/// Installs a core into `cores_dir()` and returns its path. A version is
/// installed one time: the same version must be the same file.
pub fn install(bytes: &[u8]) -> Result<PathBuf> {
    let (major, minor) = core_version(bytes).context("the file is not a Plinth core (no `plinth-core` section)")?;
    link::layout(bytes).context("the file is not a usable Plinth core")?;
    let dir = cores_dir().join(format!("{major}.{minor}"));
    let path = dir.join("core.wasm");
    if let Ok(existing) = std::fs::read(&path) {
        if existing == bytes {
            return Ok(path);
        }
        bail!("core {major}.{minor} is installed already with other contents ({})", path.display());
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Picks the core that an app module needs from the installed cores, links
/// the app into it, and returns a component that is ready to run.
pub fn link_app(app: &[u8]) -> Result<Vec<u8>> {
    let need = split::app_core_version(app).context("app.wasm does not name its core (no `plinth-core` section)")?;
    let cores = installed();
    let Some(core) = select(&cores, need) else {
        let have: Vec<String> = cores.iter().map(|c| format!("{}.{}", c.version.0, c.version.1)).collect();
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
