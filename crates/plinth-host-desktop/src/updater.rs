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
//!   downloaded.plnt  the newest verified package that the user accepted
//!                    (may not exist yet)
//!   pending.plnt     a newer verified package that the next start checks
//!                    for new capabilities before it runs (may not exist)
//! ```
//!
//! ## Re-consent (`docs/UPDATES.md` §6 "Capabilities")
//!
//! `check` stores a new version as `pending.plnt`. At the next start,
//! `prepare_start` compares the capabilities of that version with the
//! capabilities of the embedded package (the user accepted these when
//! they installed the executable) and with the decisions that the user
//! made for earlier updates. It uses the same rule as the Hub
//! (`plinth_hub::added_capabilities`, `plinth_hub::undecided`,
//! `docs/HUB.md` §7.3 step 3):
//!
//! - No new Medium or High capability: the pending version becomes the
//!   downloaded version, with no question.
//! - A new Medium or High capability with no decision: the host shows the
//!   consent window for these capabilities only. Continue records the
//!   decisions (`accept`) and runs the new version; a capability set to
//!   "Don't allow" is refused in the policy. Cancel (`decline`) keeps the
//!   previous version. The host does not ask again for the same version;
//!   it asks again only when a newer version arrives.
//! - A removed capability needs no consent.
//!
//! `<updates dir>` is `PLINTH_UPDATES_DIR`, or `%LOCALAPPDATA%\plinth\
//! updates` (next to the Hub data dir, `docs/UPDATES.md` §8 step 4)
//! elsewhere `$XDG_DATA_HOME/plinth/updates` (or
//! `~/.local/share/plinth/updates`).

use anyhow::{Context as _, Result, bail};
use plinth_package::feed::{Feed, FeedVersion};
use plinth_hub::Decision;
use plinth_package::{ApplyPolicy, Package, is_newer};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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

fn pending_path(dir: &Path) -> PathBuf {
    dir.join("pending.plnt")
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
    /// The version at `pending.plnt`: newer than the downloaded version,
    /// verified, and not yet started. `prepare_start` makes it the
    /// downloaded version, after consent if it adds a capability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_version: Option<String>,
    /// A pending version that the user refused on the consent window
    /// (Cancel). The host does not ask again for this version; a newer
    /// version replaces it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declined_version: Option<String>,
    /// The user's decisions for the capabilities that an update added, by
    /// capability name. As in the Hub, a decision does not depend on the
    /// version (`docs/HUB.md` §7.3 step 3).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub grants: BTreeMap<String, Decision>,
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

// -- Re-consent for new capabilities (`docs/UPDATES.md` §6) -----------------

/// What the host must do with a new version before it runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateConsent {
    /// The new version adds no Medium or High capability without a
    /// decision: run it with no question.
    Run,
    /// Show the consent window for these capabilities (in manifest order)
    /// first.
    Ask(Vec<String>),
    /// The user already refused this version: keep the previous version,
    /// and do not ask again.
    Declined,
}

/// Decides whether a new version needs consent before it runs. `old` is
/// the capabilities of the embedded package (the user accepted them at
/// install), `new` the capabilities of the new version, `grants` the
/// decisions that the user made for earlier updates, and `declined` the
/// version the user refused, if any. The rule is the Hub's
/// (`docs/HUB.md` §7.3 step 3): only an added Medium or High capability
/// with no decision needs consent; a removed capability needs none.
pub fn update_consent(old: &[String], new: &[String], grants: &BTreeMap<String, Decision>, version: &str, declined: Option<&str>) -> UpdateConsent {
    let ask = plinth_hub::undecided(&plinth_hub::added_capabilities(old, new), |c| grants.contains_key(c));
    if ask.is_empty() {
        UpdateConsent::Run
    } else if declined == Some(version) {
        UpdateConsent::Declined
    } else {
        UpdateConsent::Ask(ask)
    }
}

/// The capabilities of `declared` that the policy must refuse when the
/// downloaded set runs: each capability that the update added (relative
/// to `old`, the embedded package) and that is not granted by the Hub's
/// rule (`plinth_hub::is_granted`): refused by the user, or Medium/High
/// with no decision. A capability of the embedded package stays granted.
pub fn refused_capabilities(old: &[String], declared: &[String], grants: &BTreeMap<String, Decision>) -> Vec<String> {
    plinth_hub::added_capabilities(old, declared).into_iter().filter(|c| !plinth_hub::is_granted(c, grants.get(c).copied())).collect()
}

/// The same as `refused_capabilities`, with the grants that are stored in
/// `dir`.
pub fn refused_for_run(dir: &Path, old: &[String], declared: &[String]) -> Vec<String> {
    refused_capabilities(old, declared, &State::load(dir).grants)
}

/// A new version that needs consent before it runs: what the consent
/// window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    pub version: String,
    pub app_name: String,
    pub publisher: String,
    /// The signer's key id, for the consent window's publisher line.
    pub signer: Option<String>,
    /// Each capability that needs a decision, with the app's reason.
    pub capabilities: Vec<(String, String)>,
}

/// Prepares this start (`docs/UPDATES.md` §6). If a pending version
/// exists and needs no consent, it becomes the downloaded version. If it
/// needs consent, this returns the request; the caller shows the consent
/// window and calls `accept` or `decline`. A pending version that is not
/// newer than the downloaded or embedded version, or that is not a valid
/// package, is removed. Call `pick` after this.
pub fn prepare_start(dir: &Path, embedded_version: &str, embedded_capabilities: &[String]) -> Option<UpdateRequest> {
    let mut state = State::load(dir);
    let version = state.pending_version.clone()?;
    let accepted = state.downloaded_version.clone().filter(|d| is_newer(d, embedded_version)).unwrap_or_else(|| embedded_version.to_owned());
    let pkg = std::fs::read(pending_path(dir)).ok().and_then(|b| Package::read(&b).ok());
    let pkg = match pkg {
        Some(pkg) if is_newer(&version, &accepted) && pkg.manifest.version == version => pkg,
        _ => {
            let _ = std::fs::remove_file(pending_path(dir));
            state.pending_version = None;
            let _ = state.save(dir);
            return None;
        }
    };
    let declared: Vec<String> = pkg.manifest.capabilities.iter().map(|c| c.name.clone()).collect();
    match update_consent(embedded_capabilities, &declared, &state.grants, &version, state.declined_version.as_deref()) {
        UpdateConsent::Run => {
            if let Err(e) = promote(dir) {
                log::warn!("cannot use the update to version {version}: {e:#}");
            }
            None
        }
        UpdateConsent::Declined => None,
        UpdateConsent::Ask(ask) => {
            let capabilities = ask
                .into_iter()
                .map(|c| {
                    let why = pkg.manifest.capabilities.iter().find(|m| m.name == c).map(|m| m.rationale.clone()).unwrap_or_default();
                    (c, why)
                })
                .collect();
            let signer = plinth_package::signature::verify(&pkg).ok().flatten().map(|s| s.key);
            Some(UpdateRequest { version, app_name: pkg.manifest.name.clone(), publisher: pkg.manifest.publisher.clone(), signer, capabilities })
        }
    }
}

/// Moves the pending package to the downloaded slot, and resets the
/// rollback counter for it.
fn promote(dir: &Path) -> Result<()> {
    let mut state = State::load(dir);
    let version = state.pending_version.clone().context("no pending version")?;
    std::fs::rename(pending_path(dir), package_path(dir)).with_context(|| format!("rename to {}", package_path(dir).display()))?;
    state.downloaded_version = Some(version);
    state.pending_version = None;
    state.declined_version = None;
    state.failures = 0;
    state.rolled_back = false;
    state.save(dir)
}

/// The user accepted the update to `version` on the consent window:
/// records each decision (`true` is allowed), and makes the pending
/// version the downloaded version. The next `pick` runs it.
pub fn accept(dir: &Path, version: &str, decisions: &[(String, bool)]) -> Result<()> {
    let mut state = State::load(dir);
    if state.pending_version.as_deref() != Some(version) {
        bail!("version {version} is not the pending update");
    }
    for (capability, allowed) in decisions {
        state.grants.insert(capability.clone(), if *allowed { Decision::Allowed } else { Decision::Refused });
    }
    state.save(dir)?;
    promote(dir)
}

/// The user refused the update to `version` on the consent window
/// (Cancel): the previous version keeps running, and the host does not
/// ask again for this version. A newer version from the feed replaces it
/// (`check`) and is asked for again.
pub fn decline(dir: &Path, version: &str) -> Result<()> {
    let mut state = State::load(dir);
    if state.pending_version.as_deref() != Some(version) {
        bail!("version {version} is not the pending update");
    }
    state.declined_version = Some(version.to_owned());
    state.save(dir)
}

// -- `apply = "prompt"` (`docs/UPDATES.md` §5) -------------------------------

/// Whether the host shows the restart prompt after a feed check, and for
/// which version: only for `apply = "prompt"`, and only when the check
/// stored a newer version. `next-start` (and `live`, which `plinth build`
/// refuses) never prompt.
pub fn prompt_for(apply: ApplyPolicy, outcome: &CheckOutcome) -> Option<String> {
    match (apply, outcome) {
        (ApplyPolicy::Prompt, CheckOutcome::Updated(version)) => Some(version.clone()),
        _ => None,
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
    /// A newer version was verified and stored as the pending version; the
    /// next start runs it (after consent, if it adds a capability).
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
    // The baseline is the newest version this host has: pending (also
    // a declined one, so that the same version is not offered again),
    // downloaded, or running.
    let state = State::load(dir);
    let mut baseline = current_version;
    for v in [state.downloaded_version.as_deref(), state.pending_version.as_deref()].into_iter().flatten() {
        if is_newer(v, baseline) {
            baseline = v;
        }
    }
    let Some(entry) = feed.newest(channel) else { return Ok(CheckOutcome::UpToDate) };
    if !is_newer(&entry.version, baseline) {
        return Ok(CheckOutcome::UpToDate);
    }
    download_and_store(dir, feed_url, entry, &fetch)?;
    // The new version waits in `pending.plnt`; `prepare_start` checks
    // its capabilities at the next start (`docs/UPDATES.md` §6).
    let mut state = State::load(dir);
    state.pending_version = Some(entry.version.clone());
    state.declined_version = None;
    state.save(dir)?;
    Ok(CheckOutcome::Updated(entry.version.clone()))
}

/// Downloads and verifies `entry`'s package (and its core, when the feed
/// names one this host does not already have, SPEC.md §10.5), then stores
/// the package at `pending.plnt`. The core, once the project core key
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
    let tmp = dir.join(format!(".pending.plnt.tmp-{}", std::process::id()));
    std::fs::write(&tmp, &bytes).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, pending_path(dir)).with_context(|| format!("rename to {}", pending_path(dir).display()))?;
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

    fn package(identity: &PublisherIdentity, id: &str, version: &str, caps: &[&str]) -> (Vec<u8>, Package) {
        let wasm = b"\0asm fake component".to_vec();
        let mut toml = format!("id = \"{id}\"\nname = \"App\"\nversion = \"{version}\"\npublisher = \"{}\"\n", identity.name);
        for cap in caps {
            toml.push_str(&format!("\n[[capabilities]]\nname = \"{cap}\"\nrationale = \"needed for {cap}\"\n"));
        }
        let cfg = ProjectConfig::parse(&toml).unwrap();
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
        fn fetcher(&self, signer: &PublisherIdentity, version: &str) -> (impl Fn(&str) -> Result<Vec<u8>> + use<>, String) {
            self.fetcher_with(signer, version, &[])
        }

        /// Like `fetcher`, for a package that declares `caps`.
        fn fetcher_with(&self, signer: &PublisherIdentity, version: &str, caps: &[&str]) -> (impl Fn(&str) -> Result<Vec<u8>> + use<>, String) {
            let (pkg_bytes, _pkg) = package(&self.identity, &self.app_id, version, caps);
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
        assert!(pending_path(&fx.dir()).is_file());
        assert!(matches!(pick(&fx.dir(), "1.0.0"), Pick::Embedded), "a pending version does not run before prepare_start");
        assert_eq!(prepare_start(&fx.dir(), "1.0.0", &[]), None, "no new capability: no question");
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
        assert_eq!(prepare_start(&dir, "1.0.0", &[]), None);
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
        assert_eq!(prepare_start(&dir, "1.0.0", &[]), None);
        let picked = match pick(&dir, "1.0.0") {
            Pick::Downloaded(p) => Pick::Downloaded(p),
            Pick::Embedded => panic!(),
        };
        report_failure(&dir, &picked);
        report_success(&dir, &picked);
        report_failure(&dir, &picked);
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Downloaded(_)), "the reset means this is only the first failure again");
    }

    // -- Re-consent (`docs/UPDATES.md` §6 "Capabilities") ----------------

    fn caps(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn version_of(picked: &Pick) -> String {
        match picked {
            Pick::Downloaded(p) => Package::read(&std::fs::read(p).unwrap()).unwrap().manifest.version,
            Pick::Embedded => "embedded".into(),
        }
    }

    #[test]
    fn update_consent_rules() {
        let none = BTreeMap::new();
        let old = caps(&["store.kv", "clipboard.read"]);
        // The same capabilities, or fewer: no question.
        assert_eq!(update_consent(&old, &old, &none, "1.1.0", None), UpdateConsent::Run);
        assert_eq!(update_consent(&old, &caps(&["store.kv"]), &none, "1.1.0", None), UpdateConsent::Run, "a removed capability needs no consent");
        // An added Low-risk capability is granted by default (as in the Hub).
        assert_eq!(update_consent(&old, &caps(&["store.kv", "clipboard.read", "clipboard.write"]), &none, "1.1.0", None), UpdateConsent::Run);
        // An added Medium capability (and an unknown one) needs a decision, in manifest order.
        let new = caps(&["net", "store.kv", "net.local", "clipboard.read"]);
        assert_eq!(update_consent(&old, &new, &none, "1.1.0", None), UpdateConsent::Ask(caps(&["net", "net.local"])));
        // A decision from an earlier update counts, allowed or refused.
        let grants = BTreeMap::from([("net".to_string(), Decision::Allowed), ("net.local".to_string(), Decision::Refused)]);
        assert_eq!(update_consent(&old, &new, &grants, "1.1.0", None), UpdateConsent::Run);
        let some = BTreeMap::from([("net".to_string(), Decision::Allowed)]);
        assert_eq!(update_consent(&old, &new, &some, "1.1.0", None), UpdateConsent::Ask(caps(&["net.local"])));
        // A declined version is not asked again; a newer one is.
        assert_eq!(update_consent(&old, &new, &none, "1.1.0", Some("1.1.0")), UpdateConsent::Declined);
        assert_eq!(update_consent(&old, &new, &none, "1.2.0", Some("1.1.0")), UpdateConsent::Ask(caps(&["net", "net.local"])));
    }

    #[test]
    fn refused_capabilities_rules() {
        let old = caps(&["clipboard.read"]);
        let declared = caps(&["clipboard.read", "clipboard.write", "net", "net.local", "hub.manage"]);
        let grants = BTreeMap::from([("net".to_string(), Decision::Allowed), ("net.local".to_string(), Decision::Refused)]);
        // `clipboard.read` came with the install; `clipboard.write` is Low;
        // `net` is allowed; `net.local` is refused; `hub.manage` has no decision.
        assert_eq!(refused_capabilities(&old, &declared, &grants), caps(&["net.local", "hub.manage"]));
    }

    #[test]
    fn prompt_only_for_apply_prompt_and_a_new_version() {
        let updated = CheckOutcome::Updated("1.1.0".into());
        assert_eq!(prompt_for(ApplyPolicy::Prompt, &updated), Some("1.1.0".into()));
        assert_eq!(prompt_for(ApplyPolicy::Prompt, &CheckOutcome::UpToDate), None);
        assert_eq!(prompt_for(ApplyPolicy::NextStart, &updated), None);
        assert_eq!(prompt_for(ApplyPolicy::Live, &updated), None);
    }

    #[test]
    fn a_new_capability_asks_and_cancel_keeps_the_previous_version() {
        let fx = Fixture::new();
        let dir = fx.dir();
        let embedded = caps(&["store.kv"]);
        // 1.1.0 adds no capability: it runs with no question.
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.1.0", &["store.kv"]);
        check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        assert_eq!(prepare_start(&dir, "1.0.0", &embedded), None);
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.1.0");

        // 1.2.0 adds `net.local` (Medium): the next start asks.
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.2.0", &["store.kv", "net.local"]);
        assert_eq!(check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap(), CheckOutcome::Updated("1.2.0".into()));
        let request = prepare_start(&dir, "1.0.0", &embedded).expect("consent for net.local");
        assert_eq!(request.version, "1.2.0");
        assert_eq!(request.capabilities, vec![("net.local".to_string(), "needed for net.local".to_string())]);
        assert_eq!(request.signer, Some(fx.identity.key_id()));
        // Until the user decides, the previous version runs.
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.1.0");

        // Cancel: the previous version keeps running, and 1.2.0 is not offered again.
        decline(&dir, "1.2.0").unwrap();
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.1.0");
        assert_eq!(prepare_start(&dir, "1.0.0", &embedded), None, "a declined version is not asked again");
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.2.0", &["store.kv", "net.local"]);
        assert_eq!(check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap(), CheckOutcome::UpToDate);

        // A newer version asks again; Continue runs it with the decision.
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.3.0", &["store.kv", "net.local"]);
        assert_eq!(check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap(), CheckOutcome::Updated("1.3.0".into()));
        let request = prepare_start(&dir, "1.0.0", &embedded).expect("asked again for a newer version");
        assert_eq!(request.version, "1.3.0");
        accept(&dir, "1.3.0", &[("net.local".into(), true)]).unwrap();
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.3.0");
        assert!(refused_for_run(&dir, &embedded, &caps(&["store.kv", "net.local"])).is_empty());

        // The decision stays: a later version with the same capability runs with no question.
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.4.0", &["net.local"]);
        check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        assert_eq!(prepare_start(&dir, "1.0.0", &embedded), None);
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.4.0");
    }

    #[test]
    fn continue_with_dont_allow_runs_the_update_without_that_capability() {
        let fx = Fixture::new();
        let dir = fx.dir();
        let (fetch, feed_url) = fx.fetcher_with(&fx.identity, "1.1.0", &["clipboard.read"]);
        check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        let request = prepare_start(&dir, "1.0.0", &[]).expect("consent for clipboard.read");
        assert_eq!(request.capabilities.len(), 1);
        // With nothing downloaded before, the embedded version runs until the user decides.
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Embedded));
        accept(&dir, "1.1.0", &[("clipboard.read".into(), false)]).unwrap();
        assert_eq!(version_of(&pick(&dir, "1.0.0")), "1.1.0");
        assert_eq!(refused_for_run(&dir, &[], &caps(&["clipboard.read"])), caps(&["clipboard.read"]));
        // Accept and decline only act on the pending version.
        assert!(accept(&dir, "1.1.0", &[]).is_err());
        assert!(decline(&dir, "9.9.9").is_err());
    }

    #[test]
    fn a_bad_pending_package_is_removed() {
        let fx = Fixture::new();
        let dir = fx.dir();
        let (fetch, feed_url) = fx.fetcher(&fx.identity, "1.1.0");
        check(&dir, &fx.app_id, &feed_url, "stable", "1.0.0", &fx.identity.key_id(), fetch).unwrap();
        std::fs::write(pending_path(&dir), b"not a package").unwrap();
        assert_eq!(prepare_start(&dir, "1.0.0", &[]), None);
        assert!(!pending_path(&dir).exists());
        assert!(matches!(pick(&dir, "1.0.0"), Pick::Embedded));
    }
}
