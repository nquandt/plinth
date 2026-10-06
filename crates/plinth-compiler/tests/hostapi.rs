//! End-to-end test of the M2 host APIs (SPEC.md §8.5): a small app that
//! uses `setTimeout` and `store.kv`, compiled with the `store.kv`
//! capability declared, run in wasmtime, with its timer fired manually.

use plinth_compiler::driver::MemFs;
use plinth_protocol::ControlKind;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Clipboard, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::Tree;
use std::time::{Duration, Instant};

const APP: &str = "\
import { app, signal, Screen, Text } from \"plinth:ui\";
import { kv } from \"plinth:store\";
import { setTimeout } from \"plinth:time\";

function Home() {
  const message = signal(\"waiting\");

  kv.set(\"greeting\", \"hello from kv\");

  setTimeout(() => {
    const v = kv.get(\"greeting\");
    message.set(v === null ? \"missing\" : v);
  }, 10);

  return (
    <Screen title=\"Home\">
      <Text>{message()}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

fn build() -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let caps = vec!["store.kv".to_string()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("hostapi test app has errors:\n{}", diags.join("\n")))
}

fn text_of(tree: &Tree) -> String {
    let mut stack: Vec<_> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let Some(node) = tree.get(id) else { continue };
        if node.kind == Some(ControlKind::Text) {
            return node.text.clone().unwrap_or_default();
        }
        stack.extend(node.children.iter().copied());
    }
    panic!("no Text node in the tree");
}

/// `setTimeout` + `store.kv`: the timer callback reads back the value that
/// the component wrote before scheduling it, and updates a signal that a
/// `Text` child reads.
#[test]
fn timer_and_kv_round_trip() {
    let artifact = build();
    let runner = Runner::new().unwrap();
    let policy = Policy::new(["store.kv".to_string()]);
    let kv = plinth_runner_wasmtime::kv::Kv::in_memory();
    let clipboard: Box<dyn Clipboard> = Box::new(MemoryClipboard::default());
    let mut guest = runner.load_with_policy(&artifact.component, Limits::default(), policy, kv, clipboard).unwrap();

    let mut tree = Tree::new();
    let commits = guest.init(&[]).unwrap();
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    assert_eq!(text_of(&tree), "waiting", "before the timer fires");

    // No timer is due yet.
    assert!(guest.next_timer_deadline().is_some(), "setTimeout should have scheduled a timer");
    let commits = guest.fire_due_timers(Instant::now()).unwrap();
    assert!(commits.is_empty(), "the timer is not due for 10ms yet");

    // Fire it after its deadline.
    let commits = guest.fire_due_timers(Instant::now() + Duration::from_millis(20)).unwrap();
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    assert!(!commits.is_empty(), "the due timer should have committed an update");
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    assert_eq!(text_of(&tree), "hello from kv", "after the timer reads kv back");

    // One-shot: firing again later does nothing more.
    let commits = guest.fire_due_timers(Instant::now() + Duration::from_secs(1)).unwrap();
    assert!(commits.is_empty(), "a one-shot timer must not fire twice");
}

/// `store.kv` calls trap when the capability is not declared (SPEC.md
/// §8.5: "a denied call returns `error.denied(reason)` and never traps"
/// for the host; the guest's current lowering turns a denial into a trap,
/// which is the observable, documented behavior until a catchable error
/// type exists, see the coordinator report).
#[test]
fn kv_without_the_capability_traps() {
    let fs = MemFs::default().with(
        "app/main.tsx",
        "import { app, Screen, Text } from \"plinth:ui\";\nimport { kv } from \"plinth:store\";\nfunction Home() { kv.set(\"a\", \"b\"); return <Screen title=\"Home\"><Text>hi</Text></Screen>; }\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n",
    );
    // Compiled with the capability declared (so it is not a compile
    // error), but run with a policy that denies it.
    let caps = vec!["store.kv".to_string()];
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).unwrap();
    let artifact = artifact.unwrap_or_else(|| panic!("{}", front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>().join("\n")));

    let runner = Runner::new().unwrap();
    let policy = Policy::new(Vec::<String>::new()); // store.kv not declared to the runtime policy
    let kv = plinth_runner_wasmtime::kv::Kv::in_memory();
    let clipboard: Box<dyn Clipboard> = Box::new(MemoryClipboard::default());
    let mut guest = runner.load_with_policy(&artifact.component, Limits::default(), policy, kv, clipboard).unwrap();
    assert!(guest.init(&[]).is_err(), "a denied kv.set should trap, not silently succeed");
}

/// The compiler rejects a `plinth:store` call when `plinth.toml` (here,
/// the capabilities passed to the compiler) does not declare `store.kv`
/// (SPEC.md §11).
#[test]
fn undeclared_capability_is_a_compile_error() {
    let fs = MemFs::default().with(
        "app/main.tsx",
        "import { app, Screen, Text } from \"plinth:ui\";\nimport { kv } from \"plinth:store\";\nfunction Home() { kv.set(\"a\", \"b\"); return <Screen title=\"Home\"><Text>hi</Text></Screen>; }\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n",
    );
    let front = plinth_compiler::driver::frontend_with_capabilities(&fs, &[]);
    assert!(front.diags.iter().any(|d| d.code == "PL1007"), "expected PL1007, got: {:?}", front.diags.iter().map(|d| d.code).collect::<Vec<_>>());
}
