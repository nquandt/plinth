//! `plinth publish [<app dir>] [--registry <dir>] [--with-core] [--sync] [--check]`
//! (`docs/REGISTRY-HOSTING.md` §2, `docs/PRODUCTION.md` T6): one command from
//! an app folder to a registry folder. It builds with `--sign --release` into
//! `<registry>/incoming/<name>.plnt`, then runs the code of `plinth registry
//! build` on the registry folder.
//!
//! The registry folder comes from `--registry`, then `PLINTH_REGISTRY_DIR`,
//! then `registry_dir` in `publish.toml`, then `~/plinth-registry`.

use anyhow::{Context as _, Result, bail};
use plinth_package::ProjectConfig;
use std::path::{Path, PathBuf};

/// `publish.toml`, next to the `publisher` folder of the key
/// (`%APPDATA%\plinth\publish.toml` on Windows,
/// `~/.config/plinth/publish.toml` elsewhere).
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishConfig {
    /// The registry folder (a leading `~/` is the home folder).
    pub registry_dir: Option<String>,
    /// A shell command that `--sync` runs after the publish. No default.
    pub sync_command: Option<String>,
    /// The URL where the registry is served. Used for the install hint.
    pub registry_url: Option<String>,
}

pub struct Options {
    pub app_dir: PathBuf,
    /// `--registry`.
    pub registry: Option<PathBuf>,
    pub with_core: bool,
    pub sync: bool,
    /// Dry run: builds and validates, writes nothing to the registry.
    pub check: bool,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from)
}

/// The path of `publish.toml`: `PLINTH_PUBLISH_CONFIG`, or the `plinth`
/// config folder (the parent of the publisher key folder).
pub fn config_path() -> PathBuf {
    if let Some(p) = std::env::var_os("PLINTH_PUBLISH_CONFIG") {
        return PathBuf::from(p);
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| home().map(|h| h.join(".config")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("plinth").join("publish.toml")
}

fn load_config(path: &Path) -> Result<PublishConfig> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).with_context(|| format!("read {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PublishConfig::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", path.display())),
    }
}

fn expand_home(path: &str) -> PathBuf {
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        Some(rest) => home().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    }
}

/// The registry folder: `--registry`, `PLINTH_REGISTRY_DIR`, `registry_dir`
/// in the config, or `~/plinth-registry`.
pub fn registry_dir(flag: Option<&Path>, config: &PublishConfig) -> PathBuf {
    if let Some(dir) = flag {
        return dir.to_path_buf();
    }
    if let Some(dir) = std::env::var_os("PLINTH_REGISTRY_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = config.registry_dir.as_deref().filter(|d| !d.is_empty()) {
        return expand_home(dir);
    }
    home().unwrap_or_else(std::env::temp_dir).join("plinth-registry")
}

fn temp_stage() -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::env::temp_dir().join(format!("plinth-publish-{}-{nanos}", std::process::id()))
}

pub fn run(options: &Options) -> Result<bool> {
    if options.check && options.sync {
        bail!("`--check` writes nothing, so it cannot run `--sync`; use one of them");
    }
    let config_file = config_path();
    let config = load_config(&config_file)?;
    let registry = registry_dir(options.registry.as_deref(), &config);
    let sync_command = config.sync_command.as_deref().filter(|c| !c.trim().is_empty());
    if options.sync && sync_command.is_none() {
        bail!("`--sync` needs `sync_command` in {} (a shell command that uploads the registry folder); nothing was changed", config_file.display());
    }

    let text = std::fs::read_to_string(options.app_dir.join("plinth.toml"))
        .with_context(|| format!("{} has no plinth.toml; give the app folder as `plinth publish <app dir>`", options.app_dir.display()))?;
    let project = ProjectConfig::parse(&text)?;
    let identity = plinth_package::publisher::load()?;
    if project.publisher != identity.name {
        bail!(
            "cannot publish: `publisher` in plinth.toml is `{}`, but the signing key is `{}` ({}); change `publisher`, or use the other key with PLINTH_PUBLISHER_DIR",
            project.publisher,
            identity.name,
            plinth_package::publisher::publisher_dir().display()
        );
    }
    let dir = std::fs::canonicalize(&options.app_dir).with_context(|| format!("find {}", options.app_dir.display()))?;
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("app").to_owned();
    println!("publish {} ({}@{}) as {} to {}", name, project.id, project.version, identity.name, registry.display());

    // Stage 1: build into a scratch folder and run the registry build there,
    // with a copy of the app document of the registry. Every check runs, and
    // the registry stays as it is.
    let stage = temp_stage();
    let result = stage_and_check(&dir, &name, &project.id, &registry, &stage);
    let (staged, summary) = match result {
        Ok(Some(done)) => done,
        Ok(None) => {
            std::fs::remove_dir_all(&stage).ok();
            return Ok(false);
        }
        Err(e) => {
            std::fs::remove_dir_all(&stage).ok();
            return Err(e);
        }
    };

    if options.check {
        std::fs::remove_dir_all(&stage).ok();
        println!("check only: the registry at {} is not changed. It would:", registry.display());
        for (id, version) in &staged.added {
            println!("  add {id}@{version}");
        }
        for (id, version) in &staged.unchanged {
            println!("  keep {id}@{version} (the same package is already there)");
        }
        if options.with_core {
            println!("  copy the built-in core to cores/");
        }
        return Ok(true);
    }

    // Stage 2: the same package goes into `incoming/`, then the registry build.
    let incoming = registry.join("incoming");
    std::fs::create_dir_all(&incoming).with_context(|| format!("create {}", incoming.display()))?;
    let target = incoming.join(format!("{name}.{}", plinth_package::EXTENSION));
    let copied = std::fs::copy(stage.join("incoming").join(format!("{name}.{}", plinth_package::EXTENSION)), &target);
    std::fs::remove_dir_all(&stage).ok();
    copied.with_context(|| format!("write {}", target.display()))?;
    println!("built {} {summary}", target.display());
    crate::registry_cmd::build(&registry, options.with_core, Vec::new())?;

    if let Some(url) = config.registry_url.as_deref().filter(|u| !u.is_empty()) {
        let url = url.trim_end_matches('/');
        println!("registry URL: {url}");
        println!("to install: plinth hub source add <name> {url}   then   plinth hub install {}", project.id);
    }
    if options.sync {
        let command = sync_command.unwrap_or_default();
        println!("sync: {command}");
        let status = shell(command).env("PLINTH_REGISTRY_DIR", &registry).status().with_context(|| format!("run `{command}`"))?;
        if !status.success() {
            bail!("sync_command failed ({status})");
        }
    }
    Ok(true)
}

/// Builds the app into `<stage>/incoming/` and runs the registry build on
/// `stage` with a copy of the app document of `registry`. `Ok(None)` when
/// the app has errors.
fn stage_and_check(dir: &Path, name: &str, id: &str, registry: &Path, stage: &Path) -> Result<Option<(plinth_registry::build::Report, String)>> {
    let out = stage.join("incoming").join(format!("{name}.{}", plinth_package::EXTENSION));
    // The scratch path is of no use to the user: the caller prints the registry path.
    let Some((_, summary)) = crate::build_package_quiet(dir, Some(out), true, true)? else { return Ok(None) };
    let doc = Path::new("apps").join(id).join("index.json");
    if registry.join(&doc).exists() {
        let to = stage.join(&doc);
        std::fs::create_dir_all(to.parent().unwrap())?;
        std::fs::copy(registry.join(&doc), &to).with_context(|| format!("read {}", registry.join(&doc).display()))?;
    }
    let report = plinth_registry::build::build(stage, &plinth_registry::build::Options::default())
        .with_context(|| format!("the registry at {} would refuse this package", registry.display()))?;
    Ok(Some((report, summary)))
}

fn shell(command: &str) -> std::process::Command {
    if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parses_and_refuses_unknown_keys() {
        let c: PublishConfig = toml::from_str("registry_dir = \"~/r\"\nsync_command = \"echo hi\"\nregistry_url = \"https://x.example\"").unwrap();
        assert_eq!(c.sync_command.as_deref(), Some("echo hi"));
        assert!(toml::from_str::<PublishConfig>("registy_dir = \"x\"").is_err());
    }

    #[test]
    fn flag_beats_config() {
        let c = PublishConfig { registry_dir: Some("/from/config".into()), ..Default::default() };
        assert_eq!(registry_dir(Some(Path::new("/flag")), &c), PathBuf::from("/flag"));
    }
}
