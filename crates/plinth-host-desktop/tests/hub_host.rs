//! The Hub host (`docs/HUB.md` §4.2, §7.3, phase H3 step 2): how the host
//! opens a library app (`plan_launch`, `apply_consent`), and the poll loop
//! that opens a window for each app that a Hub UI guest launches. The
//! window test uses gpui's `TestAppContext` (no GPU, no real window).

use gpui::TestAppContext;
use plinth_compiler::driver::{DiskFs, MemFs};
use plinth_host_desktop::{HostApp, HubHost, LaunchPlan, PreparedApp, apply_consent, open_prepared, plan_launch, poll_hub_launches};
use plinth_runner_wasmtime::Runner;
use plinth_runner_wasmtime::policy::Policy;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

fn temp_hub() -> plinth_hub::Hub {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    plinth_hub::Hub::open(std::env::temp_dir().join(format!("plinth-hub-host-test-{}-{t}-{n}", std::process::id()))).unwrap()
}

/// `examples/counter` as a `.plnt` with `id` and the declared `caps`
/// (name, reason). The counter reaches none of them, so it links.
fn package(id: &str, version: &str, caps: &[(&str, &str)]) -> Vec<u8> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/counter");
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&DiskFs { root }, &[]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("counter has errors:\n{}", diags.join("\n"))
    });
    let mut toml = format!("id = \"{id}\"\nname = \"Counter\"\nversion = \"{version}\"\npublisher = \"me\"\n");
    for (name, why) in caps {
        toml.push_str(&format!("\n[[capabilities]]\nname = \"{name}\"\nrationale = \"{why}\"\n"));
    }
    let cfg = plinth_package::ProjectConfig::parse(&toml).unwrap();
    let manifest = cfg.manifest("1.0", "plinth-rt/1.0", None, &artifact.app);
    plinth_package::Package { manifest, component: artifact.app, assets: Vec::new(), signature: None }.write().unwrap()
}

#[test]
fn an_app_with_every_capability_decided_is_ready() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.counter", "0.1.0", &[("store.kv", "count")])).unwrap();
    match plan_launch(&hub, "com.example.counter").unwrap() {
        LaunchPlan::Ready(prepared) => {
            assert_eq!(prepared.app.app_id, "com.example.counter");
            assert!(!prepared.hub_manage);
            assert_eq!(prepared.policy.check("store.kv"), Ok(()));
        }
        LaunchPlan::NeedsConsent { .. } => panic!("store.kv is Low risk: no consent needed"),
    }
}

#[test]
fn an_undecided_capability_needs_consent_and_the_decision_is_saved() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.paste", "0.1.0", &[("clipboard.read", "paste text")])).unwrap();
    let LaunchPlan::NeedsConsent { app_name, version, pending, signer, .. } = plan_launch(&hub, "com.example.paste").unwrap() else {
        panic!("clipboard.read is Medium risk: consent first");
    };
    assert_eq!(app_name, "Counter");
    assert_eq!(version, "0.1.0");
    assert_eq!(signer, None);
    assert_eq!(pending, vec![("clipboard.read".to_owned(), "paste text".to_owned())]);

    // Cancel with one version only: nothing runs.
    assert!(apply_consent(&hub, "com.example.paste", &version, None).unwrap().is_none());

    // Continue, with the capability refused: the app runs without it.
    let prepared = apply_consent(&hub, "com.example.paste", &version, Some(vec![("clipboard.read".into(), false)])).unwrap().unwrap();
    assert!(prepared.policy.check("clipboard.read").is_err());
    assert!(matches!(plan_launch(&hub, "com.example.paste").unwrap(), LaunchPlan::Ready(_)));
}

#[test]
fn a_blocked_app_does_not_launch() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.counter", "0.1.0", &[])).unwrap();
    hub.block_app("com.example.counter").unwrap();
    let err = plan_launch(&hub, "com.example.counter").err().expect("blocked");
    assert!(format!("{err:#}").contains("blocked"), "{err:#}");
    assert!(plan_launch(&hub, "com.example.missing").is_err());
}

/// `docs/HUB.md` §7.4: an app whose publisher key is blocked does not
/// launch, also when it was installed before the block.
#[test]
fn an_app_of_a_blocked_publisher_does_not_launch() {
    let hub = temp_hub();
    let identity = plinth_package::publisher::PublisherIdentity {
        name: "me".to_owned(),
        signing_key: ed25519_dalek::SigningKey::generate(&mut rand_core::OsRng),
    };
    let mut pkg = plinth_package::Package::read(&package("com.example.signed", "0.1.0", &[])).unwrap();
    pkg.signature = Some(plinth_package::signature::sign(&pkg, &identity).unwrap());
    hub.add_package(&pkg.write().unwrap()).unwrap();
    assert!(matches!(plan_launch(&hub, "com.example.signed").unwrap(), LaunchPlan::Ready(_)));
    hub.block_publisher(&identity.key_id()).unwrap();
    let err = plan_launch(&hub, "com.example.signed").err().expect("blocked");
    assert!(format!("{err:#}").contains("blocked"), "{err:#}");
    hub.unblock_publisher(&identity.key_id()).unwrap();
    assert!(matches!(plan_launch(&hub, "com.example.signed").unwrap(), LaunchPlan::Ready(_)));
}

/// `docs/HUB.md` §9.2: a pin makes the host open the pinned version.
#[test]
fn a_pinned_version_is_the_one_that_launches() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.counter", "0.1.0", &[])).unwrap();
    hub.add_package(&package("com.example.counter", "0.2.0", &[("clipboard.read", "paste")])).unwrap();
    // The newest version declares a Medium capability: consent first.
    let LaunchPlan::NeedsConsent { version, .. } = plan_launch(&hub, "com.example.counter").unwrap() else { panic!("consent first") };
    assert_eq!(version, "0.2.0");
    hub.pin("com.example.counter", Some("0.1.0".into())).unwrap();
    let LaunchPlan::Ready(prepared) = plan_launch(&hub, "com.example.counter").unwrap() else { panic!("0.1.0 needs no consent") };
    assert!(prepared.app.capabilities.is_empty());
}

/// A stand-in for the Hub UI: it asks the host to launch two apps when it
/// starts, one ready and one that needs consent.
const LAUNCHER: &str = r#"
import { app, Screen, Text } from "plinth:ui";
import { launch } from "plinth:hub";

function Home() {
  launch("com.example.counter");
  launch("com.example.paste");
  return (
    <Screen title="Launcher">
      <Text>Launching</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

fn launcher() -> PreparedApp {
    let fs = MemFs::default().with("app/main.tsx", LAUNCHER);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &["hub.manage".to_owned()]).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("launcher has errors:\n{}", diags.join("\n"))
    });
    PreparedApp {
        app: HostApp {
            component: artifact.component,
            title: "Launcher".into(),
            accent: "blue".into(),
            app_id: "dev.plinth.test-launcher".into(),
            capabilities: vec!["hub.manage".into()],
            assets: Default::default(),
            owner: plinth_host_desktop::Owner::Dev,
        },
        policy: Policy::new(["hub.manage".to_owned()]),
        hub_manage: true,
    }
}

/// `docs/HUB.md` §4.2: a launch from the Hub UI opens the app in a new
/// window of the same process; an app that needs consent opens the
/// consent window first; the Hub window stays open.
#[test]
fn launch_requests_from_the_hub_ui_open_windows() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.counter", "0.1.0", &[])).unwrap();
    hub.add_package(&package("com.example.paste", "0.1.0", &[("clipboard.read", "paste text")])).unwrap();
    let host = HubHost { hub, runner: Arc::new(Runner::new().unwrap()), sources: None };

    let mut cx = TestAppContext::single();
    cx.skip_drawing();
    cx.update(plinth_ui::init);
    let hub_window = cx.update(|cx| open_prepared(cx, &host, launcher()));
    assert_eq!(cx.windows().len(), 1);

    // One poll: the counter opens at once; the paste app needs consent, so
    // the consent window opens.
    let open = cx.update(|cx| poll_hub_launches(cx, hub_window, &host));
    assert!(open);
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 3, "the Hub, the counter and the consent window");
    // Two app windows (a `PlinthRoot` each) and the consent window.
    let app_windows = cx.windows().iter().filter(|w| w.downcast::<plinth_ui::PlinthRoot>().is_some()).count();
    assert_eq!(app_windows, 2);

    // The requests were drained: the next poll opens nothing more.
    cx.update(|cx| poll_hub_launches(cx, hub_window, &host));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 3);

    // The poll loop that `open_prepared` started stops when the Hub window
    // closes, and the other windows stay open.
    cx.update(|cx| hub_window.update(cx, |_, window, _| window.remove_window())).unwrap();
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 2);
    assert!(!cx.update(|cx| poll_hub_launches(cx, hub_window, &host)));
}

/// The poll loop itself: `open_prepared` polls a Hub UI window on a timer,
/// so a launch opens without a direct call.
#[test]
fn the_poll_loop_runs_on_its_own() {
    let hub = temp_hub();
    hub.add_package(&package("com.example.counter", "0.1.0", &[])).unwrap();
    hub.add_package(&package("com.example.paste", "0.1.0", &[])).unwrap();
    let host = HubHost { hub, runner: Arc::new(Runner::new().unwrap()), sources: None };

    let mut cx = TestAppContext::single();
    cx.skip_drawing();
    cx.update(plinth_ui::init);
    cx.update(|cx| open_prepared(cx, &host, launcher()));
    assert_eq!(cx.windows().len(), 1);
    cx.executor().advance_clock(plinth_host_desktop::HUB_LAUNCH_POLL * 2);
    cx.run_until_parked();
    assert_eq!(cx.windows().len(), 3, "the Hub and the two launched apps");
}
