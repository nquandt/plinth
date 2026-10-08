//! End-to-end tests of `plinth:clipboard` (SPEC.md §8.5, `docs/host-apis.md`
//! "clipboard"): write then read round trips through the host's
//! `MemoryClipboard` stub (never the real OS clipboard), an empty clipboard
//! reads back `null`, the compiler rejects an undeclared capability, and a
//! capability denied by the runtime policy answers `denied:undeclared`
//! without a trap.

use plinth_compiler::driver::MemFs;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Clipboard, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::Tree;
use plinth_protocol::ControlKind;

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

const ROUND_TRIP_APP: &str = "\
import { app, signal, Screen, Text } from \"plinth:ui\";
import { writeText, readText } from \"plinth:clipboard\";

function Home() {
  writeText(\"héllo 世界 \\u{1F44B}\");
  const message = signal(readText() ?? \"null\");
  return (
    <Screen title=\"Home\">
      <Text>{message()}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";

fn run(app: &str, caps: &[&str], policy_caps: &[&str]) -> String {
    let fs = MemFs::default().with("app/main.tsx", app);
    let caps: Vec<String> = caps.iter().map(|s| s.to_string()).collect();
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let artifact = artifact.unwrap_or_else(|| panic!("app has errors:\n{}", front.diags.iter().map(|d| front.sources.render(d)).collect::<Vec<_>>().join("\n")));

    let runner = Runner::new().unwrap();
    let policy = Policy::new(policy_caps.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    let kv = plinth_runner_wasmtime::kv::Kv::in_memory();
    let clipboard: Box<dyn Clipboard> = Box::new(MemoryClipboard::default());
    let mut guest = runner.load_with_policy(&artifact.component, Limits::default(), policy, kv, clipboard).unwrap();

    let mut tree = Tree::new();
    let commits = guest.init(&[]).expect("init must not trap");
    for log in guest.take_logs() {
        eprintln!("guest: {log}");
    }
    for commit in commits {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    text_of(&tree)
}

/// `writeText` then `readText` gives back the same text, including Unicode
/// (accents, CJK, and an emoji outside the BMP).
#[test]
fn clipboard_write_then_read_round_trips_unicode() {
    let text = run(ROUND_TRIP_APP, &["clipboard.write", "clipboard.read"], &["clipboard.write", "clipboard.read"]);
    assert_eq!(text, "héllo 世界 👋");
}

/// `readText` on an empty clipboard answers `null` (`docs/host-apis.md`:
/// the stub clipboard starts empty).
#[test]
fn clipboard_read_of_an_empty_clipboard_is_null() {
    let app = "\
import { app, signal, Screen, Text } from \"plinth:ui\";
import { readText } from \"plinth:clipboard\";

function Home() {
  const message = signal(readText() === null ? \"null\" : \"not-null\");
  return (
    <Screen title=\"Home\">
      <Text>{message()}</Text>
    </Screen>
  );
}

export default app({ screens: { home: { title: \"Home\", component: Home } } });
";
    let text = run(app, &["clipboard.read"], &["clipboard.read"]);
    assert_eq!(text, "null");
}

/// The compiler rejects a `plinth:clipboard` call when `plinth.toml` does
/// not declare `clipboard.write` or `clipboard.read` (SPEC.md §11, PL1007).
#[test]
fn undeclared_clipboard_capability_is_a_compile_error() {
    let fs = MemFs::default().with(
        "app/main.tsx",
        "import { app, Screen, Text } from \"plinth:ui\";\nimport { writeText } from \"plinth:clipboard\";\nfunction Home() { writeText(\"a\"); return <Screen title=\"Home\"><Text>hi</Text></Screen>; }\nexport default app({ screens: { home: { title: \"Home\", component: Home } } });\n",
    );
    let front = plinth_compiler::driver::frontend_with_capabilities(&fs, &[]);
    assert!(front.diags.iter().any(|d| d.code == "PL1007"), "expected PL1007, got: {:?}", front.diags.iter().map(|d| d.code).collect::<Vec<_>>());
}

/// `clipboard.write`/`clipboard.read` calls never trap when the runtime
/// policy denies them, even though they were declared at compile time
/// (SPEC.md §8.5): `writeText` does nothing, `readText` returns `null`, and
/// `lastError()` reports `"denied:undeclared"`.
#[test]
fn clipboard_denied_by_policy_does_not_trap() {
    let app = "\
import { app, Screen, Text } from \"plinth:ui\";
import { writeText, readText, lastError } from \"plinth:clipboard\";
function Home() {
  writeText(\"a\");
  const v = readText();
  const err = lastError();
  return <Screen title=\"Home\"><Text>{(v === null ? \"null\" : v) + \"/\" + (err === null ? \"none\" : err)}</Text></Screen>;
}
export default app({ screens: { home: { title: \"Home\", component: Home } } });
";
    // Declared at compile time, but the runtime policy declares neither
    // capability, so every call is denied without a trap.
    let text = run(app, &["clipboard.write", "clipboard.read"], &[]);
    assert_eq!(text, "null/denied:undeclared");
}
