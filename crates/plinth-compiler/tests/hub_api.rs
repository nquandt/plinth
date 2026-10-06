//! The core 1.8 `plinth:hub` calls (`docs/HUB.md` §9.1, §5.2, phase H3
//! step 2), end to end: a small app compiled with `hub.manage`, run in
//! wasmtime with a fake `HubBackend`. It checks the synchronous calls
//! (`listGroups`, `createGroup`, `setGroup`, `remove`), the asynchronous
//! ones (`search` and `install`: a request id now, a `completion` event
//! later), and that a guest without the capability gets denied results
//! and never traps.

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, Value, Writer, event, prop};
use plinth_runner_wasmtime::hub::{HubBackend, HubJob};
use plinth_runner_wasmtime::kv::Kv;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::Tree;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const APP: &str = r#"
import { app, signal, Screen, Text, Button } from "plinth:ui";
import { listGroups, createGroup, setGroup, remove, search, install, lastError } from "plinth:hub";

function Home() {
  const groups = signal("");
  const found = signal("");
  const installed = signal("");
  const refresh = () => {
    const g = listGroups();
    groups.set(g === null ? "denied " + (lastError() ?? "") : g);
  };
  refresh();
  return (
    <Screen title="Home">
      <Text>{"groups: " + groups()}</Text>
      <Text>{"found: " + found()}</Text>
      <Text>{"installed: " + installed()}</Text>
      <Button label="Make group" onPress={() => { createGroup("Work"); setGroup("com.example.a", "Work", true); refresh(); }} />
      <Button label="Remove" onPress={() => remove("com.example.a")} />
      <Button label="Search" onPress={() => search("note", (json) => found.set(json ?? "null"))} />
      <Button label="Install" onPress={() => install("com.example.b", (error) => installed.set(error ?? "ok"))} />
    </Screen>
  );
}

export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

#[derive(Default)]
struct Calls {
    groups: Vec<String>,
    members: Vec<(String, String, bool)>,
    removed: Vec<String>,
}

struct FakeHub(Arc<Mutex<Calls>>);

impl HubBackend for FakeHub {
    fn list_apps_json(&self) -> Result<String, String> {
        Ok("[]".into())
    }
    fn launch(&mut self, _id: &str) {}
    fn set_grant(&mut self, _id: &str, _capability: &str, _allowed: bool) -> Result<(), String> {
        Ok(())
    }
    fn block(&mut self, _id: &str) -> Result<(), String> {
        Ok(())
    }
    fn unblock(&mut self, _id: &str) -> Result<(), String> {
        Ok(())
    }
    fn list_groups_json(&self) -> Result<String, String> {
        let groups: Vec<String> = self.0.lock().unwrap().groups.iter().map(|g| format!("\"{g}\"")).collect();
        Ok(format!("[{}]", groups.join(",")))
    }
    fn create_group(&mut self, name: &str) -> Result<(), String> {
        self.0.lock().unwrap().groups.push(name.to_owned());
        Ok(())
    }
    fn set_group(&mut self, id: &str, group: &str, member: bool) -> Result<(), String> {
        self.0.lock().unwrap().members.push((id.to_owned(), group.to_owned(), member));
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().removed.push(id.to_owned());
        Ok(())
    }
    fn search(&self, query: &str) -> HubJob {
        let query = query.to_owned();
        Box::new(move || Ok(format!("{{\"hits\":[],\"errors\":[],\"q\":\"{query}\"}}")))
    }
    fn install(&self, id: &str) -> HubJob {
        let id = id.to_owned();
        Box::new(move || if id == "com.example.b" { Ok(id) } else { Err("not found".into()) })
    }
}

fn build(caps: &[&str]) -> plinth_compiler::Artifact {
    let fs = MemFs::default().with("app/main.tsx", APP);
    let caps: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("hub api test app has errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(policy: Policy, hub: Option<Box<dyn HubBackend>>) -> Self {
        let artifact = build(&["hub.manage"]);
        let runner = Runner::new().unwrap();
        let mut guest = runner
            .load_with_policy_and_hub(&artifact.component, Limits::default(), policy, Kv::in_memory(), Box::new(MemoryClipboard::default()), hub)
            .unwrap();
        let mut tree = Tree::new();
        for commit in guest.init(&[]).unwrap() {
            tree.apply(&commit).unwrap();
        }
        Self { guest, tree, _runner: runner }
    }

    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for commit in commits {
            self.tree.apply(&commit).unwrap();
        }
    }

    fn press(&mut self, label: &str) {
        let node = self.find(ControlKind::Button, label).unwrap_or_else(|| panic!("no button {label}"));
        let handler = self.tree.get(node).and_then(|n| n.handler(event::PRESS)).unwrap();
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        self.apply(commits);
    }

    /// Waits for the worker threads and delivers their completions, like
    /// the desktop host's poll tick (`PlinthRoot::poll_timers`).
    fn deliver_completions(&mut self, want: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = 0;
        while got < want {
            assert!(Instant::now() < deadline, "no completion after 10 s");
            for (id, result) in self.guest.poll_net_results() {
                let commits = self.guest.answer_dialog(id, result).unwrap();
                self.apply(commits);
                got += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn find(&self, kind: ControlKind, label: &str) -> Option<plinth_protocol::NodeId> {
        let mut stack: Vec<_> = self.tree.screens().map(|(_, id)| id).collect();
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id)?;
            if node.kind == Some(kind) && node.str_prop(prop::LABEL) == Some(label) {
                return Some(id);
            }
            stack.extend(node.children.iter().copied());
        }
        None
    }

    fn texts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack: Vec<_> = self.tree.screens().map(|(_, id)| id).collect();
        while let Some(id) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            if node.kind == Some(ControlKind::Text) {
                out.push(node.text.clone().unwrap_or_default());
            }
            stack.extend(node.children.iter().rev().copied());
        }
        out
    }

    fn text(&self, prefix: &str) -> String {
        self.texts().into_iter().find(|t| t.starts_with(prefix)).unwrap_or_else(|| panic!("no text {prefix}: {:?}", self.texts()))
    }
}

#[test]
fn the_app_needs_core_1_8() {
    let artifact = build(&["hub.manage"]);
    assert_eq!(plinth_link::split::app_core_version(&artifact.app), Some((1, 8)));
}

#[test]
fn groups_remove_search_and_install_reach_the_backend() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut h = Harness::start(Policy::new(["hub.manage".to_owned()]), Some(Box::new(FakeHub(calls.clone()))));
    assert_eq!(h.text("groups:"), "groups: []");

    h.press("Make group");
    assert_eq!(h.text("groups:"), "groups: [\"Work\"]");
    assert_eq!(calls.lock().unwrap().members, vec![("com.example.a".to_owned(), "Work".to_owned(), true)]);

    h.press("Remove");
    assert_eq!(calls.lock().unwrap().removed, vec!["com.example.a".to_owned()]);

    h.press("Search");
    h.press("Install");
    h.deliver_completions(2);
    assert_eq!(h.text("found:"), "found: {\"hits\":[],\"errors\":[],\"q\":\"note\"}");
    assert_eq!(h.text("installed:"), "installed: ok");
}

/// A guest whose policy refuses `hub.manage`: every call is denied, the
/// async ones complete with their denied result, and nothing traps.
#[test]
fn a_refused_guest_gets_denied_results() {
    let calls = Arc::new(Mutex::new(Calls::default()));
    let mut policy = Policy::new(["hub.manage".to_owned()]);
    policy.refuse("hub.manage");
    let mut h = Harness::start(policy, Some(Box::new(FakeHub(calls.clone()))));
    assert_eq!(h.text("groups:"), "groups: denied denied:refused");
    h.press("Make group");
    h.press("Search");
    h.press("Install");
    h.deliver_completions(2);
    assert_eq!(h.text("found:"), "found: null");
    assert_eq!(h.text("installed:"), "installed: denied:refused");
    assert!(calls.lock().unwrap().groups.is_empty());
}

/// No backend at all (an ordinary host): `unsupported`, never a trap.
#[test]
fn no_backend_is_unsupported() {
    let mut h = Harness::start(Policy::new(["hub.manage".to_owned()]), None);
    assert_eq!(h.text("groups:"), "groups: denied denied:unsupported");
    h.press("Install");
    h.deliver_completions(1);
    assert_eq!(h.text("installed:"), "installed: denied:unsupported");
}
