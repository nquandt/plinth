//! Integration test for `docs/UPDATES.md` §8 step 4: the desktop updater
//! against a *real* local HTTP server (reusing `plinth registry serve`'s
//! tiny static file server, `plinth_registry::serve`), not a fake
//! fetcher. Checks:
//!
//! 1. A host on v1.0.0 picks up v1.1.0 after `check` against the server,
//!    and runs it on the next start (`updater::prepare_start`, then
//!    `updater::pick`).
//! 2. Two failed starts in a row of the downloaded set roll back to the
//!    embedded set, and the rollback is reported.
//!
//! No `gpui` window is involved (`updater::pick`/`report_*` have no
//! `gpui` dependency), so this test stands in for "a host instance" with
//! the same state-machine calls `plinth_host_desktop::run_payload` makes
//! (`docs/UPDATES.md` §8 step 4, 5).

use plinth_host_desktop::updater::{self, CheckOutcome, Pick};
use plinth_package::feed::{self, Feed, FeedVersion};
use plinth_package::publisher::PublisherIdentity;
use plinth_package::{Package, ProjectConfig};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-updater-http-{name}-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A suffix that is unique in this process: the clock alone is not,
/// because Windows gives it in 100 ns steps and tests run in parallel.
fn rand_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
    format!("{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

fn identity(name: &str) -> PublisherIdentity {
    PublisherIdentity { name: name.into(), signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng) }
}

fn signed_package(identity: &PublisherIdentity, id: &str, version: &str) -> Vec<u8> {
    let wasm = b"\0asm fake component".to_vec();
    let cfg = ProjectConfig::parse(&format!("id = \"{id}\"\nname = \"App\"\nversion = \"{version}\"\npublisher = \"{}\"\n", identity.name)).unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/test", None, &wasm);
    let mut pkg = Package::new(manifest, wasm, Vec::new());
    pkg.signature = Some(plinth_package::signature::sign(&pkg, identity).unwrap());
    pkg.write().unwrap()
}

/// Serves a folder holding `feed.json`, `feed.json.sig` and
/// `pkg-<version>.plnt` for the given versions, all signed by `identity`.
/// Returns the feed's base URL. The folder is left on disk (as the
/// registry crate's own e2e tests do); the server thread outlives the
/// test process's own cleanup.
fn serve_feed(identity: &PublisherIdentity, app_id: &str, versions: &[&str]) -> String {
    let dir = temp_dir("feed");
    let mut feed = Feed { schema: feed::SCHEMA.into(), id: app_id.into(), channels: Default::default() };
    for version in versions {
        let bytes = signed_package(identity, app_id, version);
        let file_name = format!("pkg-{version}.plnt");
        std::fs::write(dir.join(&file_name), &bytes).unwrap();
        feed.add_version("stable", FeedVersion { version: version.to_string(), package: file_name, sha256: hex(&Sha256::digest(&bytes)), core: "1.0".into(), core_source: None });
    }
    let feed_bytes = feed.to_bytes().unwrap();
    let sig_bytes = feed::sign(&feed_bytes, identity).unwrap();
    std::fs::write(dir.join("feed.json"), &feed_bytes).unwrap();
    std::fs::write(dir.join("feed.json.sig"), &sig_bytes).unwrap();
    let port = plinth_registry::serve::serve_background(&dir, 0).unwrap();
    format!("http://127.0.0.1:{port}/feed.json")
}

#[test]
fn second_start_picks_up_a_newer_version_from_a_real_server() {
    let identity = identity("Acme");
    let app_id = "com.example.notes";
    let feed_url = serve_feed(&identity, app_id, &["1.1.0"]);

    let state_dir = updater::app_dir(&temp_dir("state"), app_id);

    // First start: nothing downloaded yet, runs the embedded set.
    assert!(matches!(updater::pick(&state_dir, "1.0.0"), Pick::Embedded));
    let outcome = updater::check(&state_dir, app_id, &feed_url, "stable", "1.0.0", &identity.key_id(), updater::http_fetch).unwrap();
    assert_eq!(outcome, CheckOutcome::Updated("1.1.0".into()));

    // Second start: v1.1.0 adds no capability, so `prepare_start` asks
    // nothing and makes it the downloaded set; then it is picked.
    assert_eq!(updater::prepare_start(&state_dir, "1.0.0", &[]), None);
    match updater::pick(&state_dir, "1.0.0") {
        Pick::Downloaded(path) => {
            let pkg = Package::read(&std::fs::read(path).unwrap()).unwrap();
            assert_eq!(pkg.manifest.version, "1.1.0");
        }
        Pick::Embedded => panic!("expected the downloaded v1.1.0 to be picked"),
    }
}

#[test]
fn two_failed_starts_roll_back_to_embedded() {
    let identity = identity("Acme");
    let app_id = "com.example.notes";
    let feed_url = serve_feed(&identity, app_id, &["1.1.0"]);

    let state_dir = updater::app_dir(&temp_dir("state"), app_id);
    updater::check(&state_dir, app_id, &feed_url, "stable", "1.0.0", &identity.key_id(), updater::http_fetch).unwrap();
    assert_eq!(updater::prepare_start(&state_dir, "1.0.0", &[]), None);

    // Start 1: picks the downloaded set, then fails (`init` trapped).
    let picked = match updater::pick(&state_dir, "1.0.0") {
        Pick::Downloaded(p) => Pick::Downloaded(p),
        Pick::Embedded => panic!(),
    };
    assert!(!updater::report_failure(&state_dir, &picked));

    // Start 2: still picks the downloaded set (one failure is not two);
    // it fails again.
    let picked = match updater::pick(&state_dir, "1.0.0") {
        Pick::Downloaded(p) => Pick::Downloaded(p),
        Pick::Embedded => panic!("one failure must not roll back yet"),
    };
    assert!(updater::report_failure(&state_dir, &picked), "the second failure in a row rolls back");

    // Start 3: rolled back to the embedded set.
    assert!(matches!(updater::pick(&state_dir, "1.0.0"), Pick::Embedded));
    let report = updater::rollback_report(&state_dir).expect("a rollback report");
    assert!(report.contains("1.1.0"));
}
