//! `plinth feed add` (`docs/UPDATES.md` §8 step 3): writes or updates an
//! update feed and its signature from a built `.plnt` package.

use anyhow::{Context as _, Result, bail};
use plinth_package::feed::{CoreSource, Feed, FeedVersion};
use plinth_package::{Package, publisher};
use sha2::{Digest, Sha256};
use std::path::Path;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `plinth feed add <feed-dir> <app.plnt> [--channel <name>] [--with-core]`.
///
/// `feed-dir` holds `feed.json` and `feed.json.sig`; it is created if it
/// does not exist. The package's signature must be present and must be
/// made with the loaded publisher identity (`plinth publisher init`) --
/// the same identity signs the feed. `--with-core` copies the package's
/// core (SPEC.md §10.5, `plinth-compiler::link::runtime`) next to the
/// feed and lists it as the core source, by digest, so a host without
/// that core yet can fetch it (`docs/UPDATES.md` §9 UP-Q4: there is no
/// project core key yet, so a host trusts this core only because the
/// signed feed names its exact digest).
pub fn add(feed_dir: &Path, package_path: &Path, channel: &str, with_core: bool) -> Result<()> {
    let bytes = std::fs::read(package_path).with_context(|| format!("read {}", package_path.display()))?;
    let pkg = Package::read(&bytes).map_err(|e| plinth_package::read_error(e, format!("{} is not a valid .plnt package", package_path.display())))?;
    let identity = publisher::load()?;
    if pkg.manifest.publisher != identity.name {
        bail!(
            "{}'s manifest publisher is `{}`, but the loaded publisher identity is `{}`",
            package_path.display(),
            pkg.manifest.publisher,
            identity.name
        );
    }
    let signer = plinth_package::signature::verify(&pkg).context("the package's signature does not check out")?;
    let Some(signer) = signer else { bail!("{} is not signed; run `plinth sign` first", package_path.display()) };
    if signer.key != identity.key_id() {
        bail!("{} is signed by `{}`, not the loaded publisher identity `{}`", package_path.display(), signer.key, identity.key_id());
    }

    std::fs::create_dir_all(feed_dir).with_context(|| format!("create {}", feed_dir.display()))?;
    let feed_path = feed_dir.join("feed.json");
    let sig_path = feed_dir.join("feed.json.sig");
    let mut feed = if feed_path.exists() {
        let existing = std::fs::read(&feed_path).with_context(|| format!("read {}", feed_path.display()))?;
        Feed::parse(&existing).with_context(|| format!("parse {}", feed_path.display()))?
    } else {
        Feed { schema: plinth_package::feed::SCHEMA.into(), id: pkg.manifest.id.clone(), channels: Default::default() }
    };
    if feed.id != pkg.manifest.id {
        bail!("{} already holds a feed for `{}`, not `{}`", feed_dir.display(), feed.id, pkg.manifest.id);
    }

    let package_file_name = format!("{}-{}.plnt", pkg.manifest.id.rsplit('.').next().unwrap_or("app"), pkg.manifest.version);
    std::fs::write(feed_dir.join(&package_file_name), &bytes).with_context(|| format!("write {}", package_file_name))?;

    let core_source = if with_core {
        let core_bytes = plinth_compiler::link::runtime();
        let core_version = plinth_compiler::cores::core_version(core_bytes).unwrap_or_default();
        let core_version_str = plinth_compiler::cores::format_version(core_version);
        let core_file_name = format!("core-{core_version_str}.wasm");
        std::fs::write(feed_dir.join(&core_file_name), core_bytes).with_context(|| format!("write {}", core_file_name))?;
        Some(CoreSource { url: core_file_name, version: core_version_str, sha256: hex(&Sha256::digest(core_bytes)) })
    } else {
        None
    };

    // The app's lowest required core version (SPEC.md §10.5), the same
    // way `plinth registry build` computes an app document's `core`
    // field (`crates/plinth-registry/src/build.rs`).
    let core = plinth_link::split::app_core_version(&pkg.component).map(|(maj, min)| format!("{maj}.{min}")).unwrap_or_default();
    feed.add_version(
        channel,
        FeedVersion { version: pkg.manifest.version.clone(), package: package_file_name.clone(), sha256: hex(&Sha256::digest(&bytes)), core, core_source },
    );

    let feed_bytes = feed.to_bytes()?;
    let sig_bytes = plinth_package::feed::sign(&feed_bytes, &identity)?;
    std::fs::write(&feed_path, &feed_bytes).with_context(|| format!("write {}", feed_path.display()))?;
    std::fs::write(&sig_path, &sig_bytes).with_context(|| format!("write {}", sig_path.display()))?;

    println!("added {}@{} to channel `{channel}` at {}", pkg.manifest.id, pkg.manifest.version, feed_path.display());
    println!("  package: {package_file_name}");
    if with_core {
        println!("  core: included");
    }
    Ok(())
}
