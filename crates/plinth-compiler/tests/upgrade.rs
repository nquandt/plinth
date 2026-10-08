//! `onUpgrade(fromVersion)` (core 1.13, `PRODUCTION.md` D4): `plinth:ui`'s
//! upgrade hook, through a real core. Each app's manifest version is set
//! with `Guest::set_app_version` (the compiled test app has no real
//! `plinth.toml`); `Guest::finish_start` is called after `init`, as the
//! desktop host does (`crates/plinth-host-desktop/src/lib.rs`). The data
//! version is stored in a real `data-version` file next to `kv.json`
//! (`Kv::open`), in a fresh temp folder per test, so a second `start` in
//! the same test sees what the first one stored. Set `PLINTH_ENGINE=wasmi`
//! to run these on wasmi.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, NodeId};
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, Runner, kv::Kv};
use plinth_ui::tree::{Node, Tree};
use std::path::Path;
use std::time::Duration;

fn compile(src: &str) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", src);
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &[]).expect("compile");
    artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    })
}

/// An app whose `onUpgrade` handler returns `void`: shows `fromVersion`
/// (or "none") and a run count in a `Text`. `throws`: the handler throws
/// instead of finishing.
const SYNC_APP: &str = r#"
import { app, signal, Screen, Text, onUpgrade } from "plinth:ui";

const from = signal("none");
const runs = signal(0);

onUpgrade((fromVersion) => {
  runs.update((n) => n + 1);
  from.set(fromVersion);
  if (THROW) {
    throw new Error("upgrade failed");
  }
});

function Home() {
  return (
    <Screen title="Home">
      <Text>{`${from()} ${runs()}`}</Text>
    </Screen>
  );
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

/// Like `SYNC_APP`, but the handler is `async`: it awaits a timer before
/// finishing (`throws`: throws before ever starting the timer).
const ASYNC_APP: &str = r#"
import { app, signal, Screen, Text, onUpgrade } from "plinth:ui";
import { setTimeout } from "plinth:time";

const from = signal("none");
const runs = signal(0);

onUpgrade(async (fromVersion) => {
  runs.update((n) => n + 1);
  from.set(fromVersion);
  if (THROW) {
    throw new Error("upgrade failed");
  }
  await new Promise<void>((resolve) => {
    setTimeout(() => {
      from.set(fromVersion + " (done)");
      resolve();
    }, 10);
  });
});

function Home() {
  return (
    <Screen title="Home">
      <Text>{`${from()} ${runs()}`}</Text>
    </Screen>
  );
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

fn app_src(template: &str, throws: bool) -> String {
    template.replace("THROW", if throws { "true" } else { "false" })
}

struct App {
    guest: Guest,
    tree: Tree,
}

/// Starts the app at `version`, against the `data-version` file in `dir`
/// (`Kv::open`'s folder), the way the desktop host does: `set_app_version`
/// before `init`, `finish_start` after a successful one.
fn start(src: &str, dir: &Path, version: &str) -> App {
    let artifact = compile(src);
    let runner = Runner::new().unwrap();
    let kv = Kv::open(dir, "com.example.upgrade").unwrap();
    let mut guest = runner
        .load_with_policy(&artifact.component, Limits::default(), Policy::allow_all(), kv, Box::new(plinth_runner_wasmtime::MemoryClipboard::default()))
        .unwrap();
    guest.set_app_version(version);
    let mut tree = Tree::new();
    let commits = guest.init(&[]).unwrap();
    guest.finish_start();
    for commit in commits {
        assert!(tree.apply(&commit).unwrap().is_empty());
    }
    App { guest, tree }
}

fn text(tree: &Tree) -> String {
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node: &Node = tree.get(id).unwrap();
        if node.kind == Some(ControlKind::Text) {
            return node.text.clone().unwrap_or_default();
        }
        stack.extend(node.children.iter());
    }
    panic!("no Text found");
}

fn stored_version(dir: &Path) -> Option<String> {
    plinth_runner_wasmtime::app_info::read(&dir.join("apps").join("com.example.upgrade").join("data-version"))
}

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("plinth-upgrade-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn first_start_runs_no_hook_and_stores_the_version() {
    let dir = tmp("first-start");
    let app = start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    assert_eq!(text(&app.tree), "none 0");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()));
}

#[test]
fn a_newer_version_runs_the_hook_with_the_old_version() {
    let dir = tmp("newer-version");
    start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    let app = start(&app_src(SYNC_APP, false), &dir, "1.1.0");
    assert_eq!(text(&app.tree), "1.0.0 1");
    assert_eq!(stored_version(&dir), Some("1.1.0".to_owned()));
}

#[test]
fn a_hook_that_throws_does_not_store_and_runs_again() {
    let dir = tmp("throws");
    start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    let mut app = start(&app_src(SYNC_APP, true), &dir, "1.1.0");
    assert_eq!(text(&app.tree), "1.0.0 1");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()), "the throw must not store the new version");
    assert_eq!(app.guest.take_errors().len(), 1, "the throw is reported as uncaught");

    // The version is still "pending": it runs again next start.
    let app2 = start(&app_src(SYNC_APP, true), &dir, "1.1.0");
    assert_eq!(text(&app2.tree), "1.0.0 1");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()));
}

#[test]
fn an_async_hook_stores_only_after_it_resolves() {
    let dir = tmp("async");
    start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    let mut app = start(&app_src(ASYNC_APP, false), &dir, "1.1.0");
    assert_eq!(text(&app.tree), "1.0.0 1");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()), "the version is not bumped before the timer fires");

    let deadline = app.guest.next_timer_deadline().expect("the hook set a timer");
    let commits = app.guest.fire_due_timers(deadline + Duration::from_millis(1)).unwrap();
    for commit in commits {
        assert!(app.tree.apply(&commit).unwrap().is_empty());
    }
    assert_eq!(text(&app.tree), "1.0.0 (done) 1");
    assert_eq!(stored_version(&dir), Some("1.1.0".to_owned()));
    assert!(app.guest.take_errors().is_empty());
}

#[test]
fn a_downgrade_runs_no_hook_and_keeps_the_higher_version() {
    let dir = tmp("downgrade");
    start(&app_src(SYNC_APP, false), &dir, "2.0.0");
    let app = start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    assert_eq!(text(&app.tree), "none 0");
    assert_eq!(stored_version(&dir), Some("2.0.0".to_owned()));
}

#[test]
fn an_app_without_a_hook_stores_the_version() {
    let dir = tmp("no-hook");
    const PLAIN: &str = r#"
import { app, Screen, Text } from "plinth:ui";
function Home() {
  return <Screen title="Home"><Text>{"hi"}</Text></Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    start(PLAIN, &dir, "1.0.0");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()));
}

#[test]
fn the_same_version_runs_no_hook() {
    let dir = tmp("same-version");
    start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    let app = start(&app_src(SYNC_APP, false), &dir, "1.0.0");
    assert_eq!(text(&app.tree), "none 0");
    assert_eq!(stored_version(&dir), Some("1.0.0".to_owned()));
}

/// An app that never calls `onUpgrade` needs no new core function because
/// of it: it declares the same core minor as before `onUpgrade` existed.
#[test]
fn an_app_without_on_upgrade_keeps_its_core_minor() {
    const PLAIN: &str = r#"
import { app, signal, isActive, Screen, Text, Button } from "plinth:ui";
const count = signal(0);
function Home() {
  return (
    <Screen title="Home">
      <Text>{`${count()} ${isActive()}`}</Text>
      <Button label="Add" onPress={() => count.set(count() + 1)} />
    </Screen>
  );
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;
    assert_eq!(compile(PLAIN).runtime, "plinth-core/1.12");
    assert_eq!(compile(&app_src(SYNC_APP, false)).runtime, "plinth-core/1.13");
}
