//! The desktop updater for a `plinth native` executable
//! (`docs/UPDATES.md` §8 step 4, 5): checks the author's feed in the
//! background, downloads and verifies a newer app (and a core, if the
//! feed lists one), stores it in a per-user folder, and runs the newest
//! *verified* set on the next start. The embedded `.plnt` is always the
//! fallback.
//!
//! This module has no `gpui` dependency, so its state machine and its
//! feed logic are tested with no window or renderer (`cargo test -p
//! plinth-host-desktop --lib`).
//!
//! ## Layout on disk
//!
//! ```text
//! <updates dir>/<app id>/
//!   state.json      the rollback state machine (`State`)
//!   downloaded.plnt  the newest verified package (may not exist yet)
//! ```
//!
//! `<updates dir>` is `PLINTH_UPDATES_DIR`, or `%LOCALAPPDATA%\plinth\
//! updates` (next to the Hub data dir, `docs/UPDATES.md` §8 step 4)
//! elsewhere `$XDG_DATA_HOME/plinth/updates` (or
//! `~/.local/share/plinth/updates`).

use anyhow::{Context as _, Result, bail};
use plinth_package::feed::{Feed, FeedVersion};
use plinth_package::{Package, is_newer};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A new set stops at start this many times in a row before the host
/// rolls back (`docs/UPDATES.md` §6).
pub const MAX_FAILURES: u32 = 2;

/// The updates directory: `PLINTH_UPDATES_DIR`, or
/// `%LOCALAPPDATA%\plinth\updates` on Windows and
/// `$XDG_DATA_HOME/plinth/updates` (or `~/.local/share/plinth/updates`)
/// elsewhere. Modeled on `plinth_hub::hub_dir`.
pub fn updates_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PLINTH_UPDATES_DIR") {
        return PathBuf::from(dir);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("plinth").join("updates")
}

/// The per-app folder under the updates directory.
pub fn app_dir(updates_dir: &Path, app_id: &str) -> PathBuf {
    updates_dir.join(app_id)
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

fn package_path(dir: &Path) -> PathBuf {
    dir.join("downloaded.plnt")
}

/// The rollback state machine's persisted state (`docs/UPDATES.md` §6).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub struct State {
    /// The version of the downloaded package at `downloaded.plnt`, once
    /// one has been verified and stored. `None` means no update has been
    /// downloaded yet (or none has ever verified).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloaded_version: Option<String>,
    /// How many times in a row the downloaded set has failed to start
    /// (`init` trapped, or the load itself failed). Reset to 0 by a
    /// successful start. At `MAX_FAILURES` the set is marked bad and the
    /// host falls back to the embedded package.
    #[serde(default)]
    pub failures: u32,
    /// A downloaded set that reached `MAX_FAILURES` and was rolled back.
    /// The host does not try it again (it also would not: a new feed
    /// check only replaces it with a *newer* version, never the same
    /// one), but this flag is what `last_failure_report` reads to tell
    /// the user.
    #[serde(default)]
    pub rolled_back: bool,
}

impl State {
    fn load(dir: &Path) -> State {
        std::fs::read(state_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let bytes = serde_json::to_vec_pretty(self)?;
        let tmp = dir.join(format!(".state.json.tmp-{}", std::process::id()));
        std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, state_path(dir)).with_context(|| format!("rename to {}", state_path(dir).display()))?;
        Ok(())
    }
}

/// Which package bytes a start should run: the embedded `.plnt` (the
/// fallback, and the default when nothing was ever downloaded, or the
/// downloaded set rolled back), or the downloaded one.
pub enum Pick {
    Embedded,
    Downloaded(PathBuf),
}

/// Chooses which set to run at this start, and records that this start is
/// an attempt of that set (so a crash mid-start, with no later
/// `report_success`/`report_failure`, still counts as a failed attempt
/// the *next* time the host starts — `report_failure` is also called
/// explicitly on a detected `init` failure, so both paths add up to the
/// same count across restarts).
///
/// `embedded_version` is the version inside the executable, used for the
/// no-downgrade check: a downloaded set older than the embedded version
/// is never picked (an old feed, or a feed that moved to an older
/// channel, must not roll a user back).
pub fn pick(dir: &Path, embedded_version: &str) -> Pick {
    let state = State::load(dir);
    let downloaded = package_path(dir);
    let Some(downloaded_version) = &state.downloaded_version else { return Pick::Embedded };
    if !downloaded.is_file() || state.failures >= MAX_FAILURES || !is_newer(downloaded_version, embedded_version) {
        return Pick::Embedded;
    }
    Pick::Downloaded(downloaded)
}

/// Records that the set `pick` returned just finished starting
/// successfully (`init` did not trap, and nothing failed to load). A
/// no-op for `Pick::Embedded` (the embedded set has no failure counter;
/// it is always available).
pub fn report_success(dir: &Path, picked: &Pick) {
    if matches!(picked, Pick::Embedded) {
        return;
    }
    let mut state = State::load(dir);
    state.failures = 0;
    let _ = state.save(dir);
}

/// Records that the set `pick` returned failed to start (`init` trapped,
/// or the load itself failed). Returns `true` once this reaches
/// `MAX_FAILURES`: the caller should tell the user (log, and the error
/// banner path) that the update was rolled back. A no-op (returns
/// `false`) for `Pick::Embedded`: the embedded set is the last resort and
/// is never rolled back away from.
pub fn report_failure(dir: &Path, picked: &Pick) -> bool {
    if matches!(picked, Pick::Embedded) {
        return false;
    }
    let mut state = State::load(dir);
    state.failures += 1;
    let rolled_back_now = state.failures >= MAX_FAILURES;
    if rolled_back_now {
        state.rolled_back = true;
    }
    let _ = state.save(dir);
    rolled_back_now
}

/// A human-readable line for the log (and the error banner, when simple)
/// about the current rollback state. `None` when there is nothing to
/// report (no update downloaded, or it is running fine).
pub fn rollback_report(dir: &Path) -> Option<String> {
    let state = State::load(dir);
    if state.rolled_back {
        Some(format!(
            "update to version {} failed to start {} times in a row; rolled back to the previous version (docs/UPDATES.md §6)",
            state.downloaded_version.as_deref().unwrap_or("?"),
            state.failures
        ))
    } else {
        None
    }
}

/// Fetches bytes over HTTP/HTTPS with a short timeout, for the feed and
/// the files it names. Kept separate from `check` so tests can supply a
/// fake fetcher with no real network.
pub fn http_fetch(url: &str) -> Result<Vec<u8>> {
    let config = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent.get(url).call().with_context(|| format!("fetch {url}"))?;
    use std::io::Read as _;
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(64 << 20)
        .read_to_end(&mut buf)
        .with_context(|| format!("read body of {url}"))?;
    Ok(buf)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolves `rel` against `base_url` (the feed's own URL): an absolute
/// `rel` (its own scheme) is returned as-is; otherwise `rel` replaces the
/// last path segment of `base_url`.
fn resolve(base_url: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_owned();
    }
    match base_url.rfind('/') {
        Some(i) => format!("{}/{rel}", &base_url[..i]),
        None => rel.to_owned(),
    }
}

/// What `check` found and did.
#[derive(Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    /// No newer, verified version than what is already downloaded (or
    /// embedded, if nothing is downloaded).
    UpToDate,
    /// A newer version was verified and stored; it will run on the next
    /// start.
    Updated(String),
}

/// Checks the feed at `feed_url`, verifies it, and downloads+stores a
/// newer version of `channel` than `current_version` (the version the
/// host currently runs: the embedded version, or a downloaded version
/// already in use). `embedded_signer_key` is the publisher key id of the
/// *embedded* package (`docs/UPDATES.md` §6: an update must carry the
/// same key, so a feed signed by a different key, even a validly signed
/// one, is refused — key continuity). `fetch` lets a test supply canned
/// bytes instead of real HTTP.
///
/// Safety checks, in order: the feed's own signature; the feed's `id`
/// matches `app_id`; the signer's key equals `embedded_signer_key`; no
/// downgrade; the package's digest; a core, only when the feed lists its
/// digest (there is no project core key yet, `docs/UPDATES.md` §9 UP-Q4,
/// so a signed feed's own digest listing stands in for it).
pub fn check(
    dir: &Path,
    app_id: &str,
    feed_url: &str,
    channel: &str,
    current_version: &str,
    embedded_signer_key: &str,
    fetch: impl Fn(&str) -> Result<Vec<u8>>,
) -> Result<CheckOutcome> {
    let feed_bytes = fetch(feed_url).context("fetch the feed")?;
    let sig_url = format!("{feed_url}.sig");
    let sig_bytes = fetch(&sig_url).context("fetch the feed signature")?;
    let signer = plinth_package::feed::verify(&feed_bytes, &sig_bytes).context("the feed's signature does not check out")?;
    if embedded_signer_key.is_empty() {
        bail!("the embedded package is unsigned; an unsigned app cannot take signed updates (no key to check continuity against)");
    }
    if signer.key != embedded_signer_key {
        bail!(
            "the feed is signed by `{}`, not the embedded package's key `{embedded_signer_key}`; refusing the update (key continuity, docs/UPDATES.md §6)",
            signer.key
        );
    }
    let feed = Feed::parse(&feed_bytes)?;
    if feed.id != app_id {
        bail!("the feed at {feed_url} is for `{}`, not `{app_id}`", feed.id);
    }
    let state = State::load(dir);
    let baseline = state.downloaded_version.as_deref().unwrap_or(current_version);
    let Some(entry) = feed.newest(channel) else { return Ok(CheckOutcome::UpToDate) };
    if !is_newer(&entry.version, baseline) {
        return Ok(CheckOutcome::UpToDate);
    }
    download_and_store(dir, feed_url, entry, &fetch)?;
    let mut state = State::load(dir);
    state.downloaded_version = Some(entry.version.clone());
    state.failures = 0;
    state.rolled_back = false;
    state.save(dir)?;
    Ok(CheckOutcome::Updated(entry.version.clone()))
}

/// Downloads and verifies `entry`'s package (and its core, when the feed
/// names one this host does not already have, SPEC.md §10.5), then stores
/// the package at `downloaded.plnt`. The core, once the project core key
/// exists (UP-Q4), will be installed the same way `plinth core install`
/// does; for now this only verifies the core's digest against the signed
/// feed's listing and leaves installing it to the author's existing
/// `--with-core` channel (this function does not install cores: see
/// `docs/UPDATES.md` for the open item).
fn download_and_store(dir: &Path, feed_url: &str, entry: &FeedVersion, fetch: &impl Fn(&str) -> Result<Vec<u8>>) -> Result<()> {
    let package_url = resolve(feed_url, &entry.package);
    let bytes = fetch(&package_url).with_context(|| format!("fetch {package_url}"))?;
    let digest = sha256_hex(&bytes);
    if digest != entry.sha256 {
        bail!("{package_url} does not match the feed's sha256 for version {} (expected {}, got {digest})", entry.version, entry.sha256);
    }
    let pkg = Package::read(&bytes).with_context(|| format!("{package_url} is not a valid .plnt package"))?;
    if pkg.manifest.version != entry.version {
        bail!("{package_url}'s manifest says version {}, the feed says {}", pkg.manifest.version, entry.version);
    }
    if let Some(core_source) = &entry.core_source {
        let core_url = resolve(feed_url, &core_source.url);
        // The digest check stands in for the (not yet built) project core
        // key: a core is accepted only when its digest is the one this
        // already-verified, signed feed lists (`docs/UPDATES.md` §9
        // UP-Q4).
        let core_bytes = fetch(&core_url).with_context(|| format!("fetch {core_url}"))?;
        let core_digest = sha256_hex(&core_bytes);
        if core_digest != core_source.sha256 {
            bail!("{core_url} does not match the feed's sha256 for core {} (expected {}, got {core_digest})", core_source.version, core_source.sha256, core_digest = core_digest);
        }
        plinth_link::cores::install(&core_bytes).with_context(|| format!("install core {}", core_source.version))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let tmp = dir.join(format!(".downloaded.plnt.tmp-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, package_path(dir)).with_context(|| format!("rename to {}", package_path(dir).display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use plinth_package::publisher::PublisherIdentity;
    use plinth_package::{ProjectConfig, feed};

    fn identity(name: &str) -> PublisherIdentity {
        PublisherIdentity { name: name.into(), signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng) }
    }

    fn package(identity: &PublisherIdentity, id: &str, version: &str) -> (Vec<u8>, Package) {
        let wasm = b"\0asm fake component".to_vec();
        let cfg = ProjectConfig::parse(&format!("id = \"{id}\"\nname = \"App\"\nversion = \"{version}\"\npublisher = \"{}\"\n", identity.name)).unwrap();
        let manifest = cfg.manifest("1.0", "plinth-rt/test", None, &wasm);
        let mut pkg = Package { manifest, component: wasm, assets: Vec::new(), signature: None };
        pkg.signature = Some(plinth_package::signature::sign(&pkg, identity).unwrap());
        (pkg.write().unwrap(), pkg)
    }

    struct Fixture {
        temp: PathBuf,
        feed_url: String,
        identity: PublisherIdentity,
        app_id: String,
    }

    impl Fixture {
        fn new() -> Fixture {
            let temp = std::env::temp_dir().join(format!("plinth-updater-test-{}-{}", std::process::id(), rand_suffix()));
            std::fs::create_dir_all(&temp).unwrap();
            Fixture { temp, feed_url: "file://feed.json".into(), identity: identity("Acme"), app_id: "com.example.notes".into() }
        }

        fn dir(&self) -> PathBuf {
            app_dir(&self.temp, &self.app_id)
        }

        /// Builds a feed with one `stable` entry for `version`, signs it
        /// with `signer`, and returns a `fetch` closure serving the feed,
        /// its signature, and the package from in-memory bytes (no real
        /// network or files on disk other than this fixture's own temp
        /// dir).
        fn fetcher(&self, signer: &PublisherIdentity, version: &str) -> (impl Fn(&str) -> Result<Vec<u8>>, String) {
            let (pkg_bytes, _pkg) = package(&self.identity, &self.app_id, version);
            let digest = sha256_hex(&pkg_bytes);
            let mut feed = feed::Feed { schema: feed::SCHEMA.into(), id: self.app_id.clone(), channels: Default::default() };
            feed.add_version(
                "stable",
                feed::FeedVersion { version: version.into(), package: "pkg.plnt".into(), sha256: digest, core: "1.0".into(), core_source: None },
            );
            let feed_bytes = feed.to_bytes().unwrap();
            let sig_bytes = feed::sign(&feed_bytes, signer).unwrap();
            let feed_url = self.feed_url.clone();
            let map = std::sync::Arc::new(std::collections::HashMap::from([
                (feed_url.clone(), feed_bytes),
                (format!("{feed_url}.sig"), sig_bytes),
                ("file://pkg.plnt".to_owned(), pkg_bytes),
            ]));
            let fetch = move |url: &str| -> Result<Vec<u8>> { map.get(url).cloned().context(format!("no fixture data for {url}")) };
            (fetch, feed_url)
        }
    }

    fn rand_suffix() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    #[test]
    fn check_downloads_a_newer_signed_version() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.1.0");
        let outcome = check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        assert_eq!(outcome, CheckOutcome::Updated("1.1.0".into()));
        assert!(package_path(&fx.dir()).is_file());
        match pick(&fx.dir(), "1.0.0") {
            Pick::Downloaded(_) => {}
            Pick::Embedded => panic!("expected the downloaded set to be picked"),
        }
    }

    #[test]
    fn check_is_up_to_date_when_the_feed_has_nothing_newer() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.0.0");
        let outcome = check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        assert_eq!(outcome, CheckOutcome::UpToDate);
    }

    #[test]
    fn check_refuses_a_feed_signed_by_a_different_key() {
        let fx = Fixture::new();
        let other = identity("Mallory");
        let (fetch, feed_url) = fx.fetcher(&other, "1.1.0");
        let err = check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap_err();
        assert!(format!("{err:#}").contains("key continuity") || format!("{err:#}").contains("not the embedded"));
    }

    #[test]
    fn check_refuses_a_tampered_package() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.1.0");
        let fetch = move |url: &str| {
            let bytes = fetch(url)?;
            Ok(if url.ends_with("pkg.plnt") { bytes.iter().map(|b| b ^ 1).collect() } else { bytes })
        };
        let err = check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap_err();
        assert!(format!("{err:#}").contains("sha256"));
    }

    #[test]
    fn check_never_downgrades() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.0.0");
        // Running version is already newer than the feed's only entry.
        let outcome = check(&fx.dir(), &fx.app_id, &feed_url, "stable", "2.0.0", &fx.identity.key_id(), fetch).unwrap();
        assert_eq!(outcome, CheckOutcome::UpToDate);
    }

    #[test]
    fn pick_falls_back_to_embedded_with_no_download() {
        let fx = Fixture::new();
        assert!(matches!(pick(&fx.dir(), "1.0.0"), Pick::Embedded));
    }

    #[test]
    fn rollback_after_two_failures() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.1.0");
        check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        let dir = fx.dir();
        let picked = match pick(&dir, "1.0.0") {
            Pick::Downloaded(p) => Pick::Downloaded(p),
            Pick::Embedded => panic!(),
        };
        assert!(!report_failure(&dir, &picked));
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Downloaded(_)), "one failure does not roll back yet");
        assert!(report_failure(&dir, &picked), "the second failure rolls back");
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Embedded), "rolled back to the embedded set");
        assert!(rollback_report(&dir).unwrap().contains("1.1.0"));
    }

    #[test]
    fn a_success_resets_the_failure_count() {
        let fx = Fixture::new();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.1.0");
        check(&fx.dir(), &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        let dir = fx.dir();
        let picked = match pick(&dir, "1.0.0") {
            Pick::Downloaded(p) => Pick::Downloaded(p),
            Pick::Embedded => panic!(),
        };
        report_failure(&dir, &picked);
        report_success(&dir, &picked);
        report_failure(&dir, &picked);
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Downloaded(_)), "the reset means this is only the first failure again");
    }
}
