//! The Hub UI (`examples/hub`, `docs/HUB.md` §4.1, phase H3 step 2),
//! driven headless in wasmtime against a fake `plinth:hub` backend that
//! keeps an in-memory library: the library list, group tabs, the text
//! filter, the app page with its capability label and grants, groups,
//! block, launch, a new group, search and install, and remove.

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::hub::{HubBackend, HubJob};
use plinth_runner_wasmtime::kv::Kv;
use plinth_runner_wasmtime::policy::Policy;
use plinth_runner_wasmtime::{Guest, Limits, MemoryClipboard, Runner};
use plinth_ui::tree::{Node, Tree};
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
struct FakeApp {
    id: String,
    name: String,
    blocked: bool,
    publisher_blocked: bool,
    groups: Vec<String>,
    /// (name, risk, description, decided, allowed)
    caps: Vec<(String, String, String, bool, bool)>,
    /// Installed versions, oldest first; the last one is the newest.
    versions: Vec<String>,
    pinned: String,
    /// The update that the last check found, or "".
    update: String,
}

impl FakeApp {
    fn new(id: &str, name: &str, groups: Vec<String>, caps: Vec<(String, String, String, bool, bool)>) -> Self {
        FakeApp {
            id: id.into(),
            name: name.into(),
            blocked: false,
            publisher_blocked: false,
            groups,
            caps,
            versions: vec!["1.0.0".into()],
            pinned: String::new(),
            update: String::new(),
        }
    }
}

#[derive(Default)]
struct Library {
    apps: Vec<FakeApp>,
    groups: Vec<String>,
    launches: Vec<String>,
}

/// The source lists Notes 1.1.0, which adds one capability.
const NOTES_UPDATE: &str = "1.1.0";

fn sync_capability() -> (String, String, String, bool, bool) {
    ("net:sync.example.com".into(), "medium".into(), "connect to sync.example.com".into(), false, false)
}

fn notes() -> FakeApp {
    FakeApp::new(
        "com.example.notes",
        "Notes",
        vec!["Work".into()],
        vec![
            ("store.kv".into(), "low".into(), "save data on this device".into(), true, true),
            ("clipboard.read".into(), "medium".into(), "read the clipboard".into(), false, false),
        ],
    )
}

fn weather() -> FakeApp {
    FakeApp::new(
        "com.example.weather",
        "Weather",
        Vec::new(),
        vec![("net:api.example.com".into(), "medium".into(), "connect to api.example.com".into(), true, true)],
    )
}

/// The source catalog: `com.example.timer` can be installed.
fn timer() -> FakeApp {
    FakeApp::new("com.example.timer", "Timer", Vec::new(), Vec::new())
}

struct FakeHub(Arc<Mutex<Library>>);

fn app_json(a: &FakeApp) -> serde_json::Value {
    let caps: Vec<_> = a
        .caps
        .iter()
        .map(|(name, risk, description, decided, allowed)| {
            json!({ "name": name, "risk": risk, "description": description, "rationale": "", "decided": decided, "allowed": allowed, "byDefault": false })
        })
        .collect();
    let running = if a.pinned.is_empty() { a.versions.last().unwrap().clone() } else { a.pinned.clone() };
    let versions: Vec<_> = a.versions.iter().rev().map(|v| json!({ "version": v, "signer": "ed25519:abc", "capabilities": [] })).collect();
    let update_caps: Vec<String> = if a.update.is_empty() { Vec::new() } else { vec![sync_capability().0] };
    json!({
        "id": a.id, "name": a.name, "version": running, "publisher": "Example", "signer": "ed25519:abc",
        "source": "main", "blocked": a.blocked, "publisherBlocked": a.publisher_blocked, "groups": a.groups,
        "capabilities": caps, "pinned": a.pinned, "versions": versions, "update": a.update,
        "updateCapabilities": update_caps,
    })
}

impl HubBackend for FakeHub {
    fn list_apps_json(&self) -> Result<String, String> {
        let lib = self.0.lock().unwrap();
        Ok(serde_json::to_string(&lib.apps.iter().map(app_json).collect::<Vec<_>>()).unwrap())
    }
    fn launch(&mut self, id: &str) {
        self.0.lock().unwrap().launches.push(id.to_owned());
    }
    fn take_launches(&mut self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap().launches)
    }
    fn set_grant(&mut self, id: &str, capability: &str, allowed: bool) -> Result<(), String> {
        let mut lib = self.0.lock().unwrap();
        let app = lib.apps.iter_mut().find(|a| a.id == id).ok_or("no app")?;
        let cap = app.caps.iter_mut().find(|c| c.0 == capability).ok_or("no capability")?;
        cap.3 = true;
        cap.4 = allowed;
        Ok(())
    }
    fn block(&mut self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().apps.iter_mut().filter(|a| a.id == id).for_each(|a| a.blocked = true);
        Ok(())
    }
    fn unblock(&mut self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().apps.iter_mut().filter(|a| a.id == id).for_each(|a| a.blocked = false);
        Ok(())
    }
    fn list_groups_json(&self) -> Result<String, String> {
        Ok(serde_json::to_string(&self.0.lock().unwrap().groups).unwrap())
    }
    fn create_group(&mut self, name: &str) -> Result<(), String> {
        let mut lib = self.0.lock().unwrap();
        if !lib.groups.iter().any(|g| g == name) {
            lib.groups.push(name.to_owned());
        }
        Ok(())
    }
    fn set_group(&mut self, id: &str, group: &str, member: bool) -> Result<(), String> {
        let mut lib = self.0.lock().unwrap();
        let app = lib.apps.iter_mut().find(|a| a.id == id).ok_or("no app")?;
        app.groups.retain(|g| g != group);
        if member {
            app.groups.push(group.to_owned());
        }
        Ok(())
    }
    fn remove(&mut self, id: &str) -> Result<(), String> {
        self.0.lock().unwrap().apps.retain(|a| a.id != id);
        Ok(())
    }
    fn search(&self, query: &str) -> HubJob {
        let query = query.to_lowercase();
        Box::new(move || {
            let hits: Vec<_> = [notes(), timer()]
                .iter()
                .filter(|a| a.name.to_lowercase().contains(&query))
                .map(|a| json!({ "id": a.id, "name": a.name, "version": "1.0.0", "description": "An app", "source": "main" }))
                .collect();
            Ok(json!({ "hits": hits, "errors": ["broken: cannot reach the source"] }).to_string())
        })
    }
    fn app_info_json(&self, id: &str) -> Result<String, String> {
        let lib = self.0.lock().unwrap();
        let app = lib.apps.iter().find(|a| a.id == id).ok_or("no app")?;
        Ok(app_json(app).to_string())
    }
    fn pin(&mut self, id: &str, version: &str) -> Result<(), String> {
        let mut lib = self.0.lock().unwrap();
        let app = lib.apps.iter_mut().find(|a| a.id == id).ok_or("no app")?;
        if !version.is_empty() && !app.versions.iter().any(|v| v == version) {
            return Err("no such version".into());
        }
        app.pinned = version.to_owned();
        Ok(())
    }
    fn block_publisher(&mut self, key: &str) -> Result<(), String> {
        assert_eq!(key, "ed25519:abc");
        self.0.lock().unwrap().apps.iter_mut().for_each(|a| a.publisher_blocked = true);
        Ok(())
    }
    fn unblock_publisher(&mut self, key: &str) -> Result<(), String> {
        assert_eq!(key, "ed25519:abc");
        self.0.lock().unwrap().apps.iter_mut().for_each(|a| a.publisher_blocked = false);
        Ok(())
    }
    fn check_updates(&self, id: &str) -> HubJob {
        assert_eq!(id, "", "the Hub UI checks every app");
        let lib = self.0.clone();
        Box::new(move || {
            let mut lib = lib.lock().unwrap();
            let mut updates = Vec::new();
            for app in lib.apps.iter_mut() {
                if app.id == "com.example.notes" && !app.versions.iter().any(|v| v == NOTES_UPDATE) {
                    app.update = NOTES_UPDATE.into();
                    updates.push(json!({ "id": app.id, "name": app.name, "current": "1.0.0", "version": NOTES_UPDATE,
                        "source": "main", "newCapabilities": [sync_capability().0], "pinned": app.pinned }));
                }
            }
            Ok(json!({ "updates": updates, "errors": [] }).to_string())
        })
    }
    fn update(&self, id: &str) -> HubJob {
        let lib = self.0.clone();
        let id = id.to_owned();
        Box::new(move || {
            let mut lib = lib.lock().unwrap();
            let app = lib.apps.iter_mut().find(|a| a.id == id).ok_or("no app")?;
            if app.update.is_empty() {
                return Ok(String::new());
            }
            app.versions.push(std::mem::take(&mut app.update));
            app.caps.push(sync_capability());
            Ok(NOTES_UPDATE.to_owned())
        })
    }
    fn install(&self, id: &str) -> HubJob {
        let lib = self.0.clone();
        let id = id.to_owned();
        Box::new(move || {
            if id != "com.example.timer" {
                return Err(format!("no configured source lists {id}"));
            }
            lib.lock().unwrap().apps.push(timer());
            Ok(id)
        })
    }
}

fn build_hub() -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/hub");
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &["hub.manage".to_owned()]).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("examples/hub has errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(lib: Arc<Mutex<Library>>) -> Self {
        let artifact = build_hub();
        let runner = Runner::new().unwrap();
        let mut guest = runner
            .load_with_policy_and_hub(
                &artifact.component,
                Limits::default(),
                Policy::new(["hub.manage".to_owned()]),
                Kv::in_memory(),
                Box::new(MemoryClipboard::default()),
                Some(Box::new(FakeHub(lib))),
            )
            .unwrap();
        let mut tree = Tree::new();
        let commits = guest.init(&[]);
        for log in guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        Self { guest, tree, _runner: runner }
    }

    fn apply(&mut self, commits: Vec<Vec<u8>>) {
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let commits = self.guest.on_event(w.as_bytes()).unwrap();
        self.apply(commits);
    }

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

    /// Nodes of `kind` in the displayed screen's subtree.
    fn find(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        let mut found = Vec::new();
        let Some(root) = self.tree.current_root() else { return found };
        let mut stack = vec![root.id];
        while let Some(id) = stack.pop() {
            let node = self.tree.get(id).unwrap();
            if node.kind == Some(kind) && pred(node) {
                found.push(id);
            }
            stack.extend(node.children.iter().rev());
        }
        found
    }

    fn one(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
        let f = self.find(kind, pred);
        assert_eq!(f.len(), 1, "expected one {kind:?} in the current screen, found {}", f.len());
        f[0]
    }

    fn labeled(&self, kind: ControlKind, label: &str) -> NodeId {
        self.one(kind, |n| n.str_prop(prop::LABEL) == Some(label))
    }

    fn row(&self, title: &str) -> NodeId {
        self.one(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some(title))
    }

    fn row_titles(&self) -> Vec<String> {
        self.find(ControlKind::Row, |_| true)
            .into_iter()
            .map(|r| self.tree.get(r).unwrap().str_prop(prop::TITLE).unwrap_or_default().to_owned())
            .collect()
    }

    fn texts(&self) -> Vec<String> {
        self.find(ControlKind::Text, |_| true).into_iter().map(|t| self.tree.get(t).unwrap().text.clone().unwrap_or_default()).collect()
    }

    fn screen_title(&self) -> String {
        self.tree.current_root().unwrap().str_prop(prop::TITLE).unwrap_or_default().to_owned()
    }

    fn primary(&mut self, name: &str) {
        // Selecting a primary screen is a host action (the sidebar); at
        // the protocol level it is the tree's displayed screen.
        let index = self
            .tree
            .primary_screens()
            .find(|(_, id)| self.tree.get(*id).unwrap().str_prop(prop::TITLE) == Some(name))
            .map(|(s, _)| s)
            .unwrap_or_else(|| panic!("no primary screen {name}"));
        self.tree.current_screen = index;
    }
}

#[test]
fn the_hub_ui_manages_the_library() {
    let lib = Arc::new(Mutex::new(Library { apps: vec![notes(), weather()], groups: vec!["Work".into()], launches: Vec::new() }));
    let mut h = Harness::start(lib.clone());
    // The Hub checks for updates when it starts (`docs/HUB.md` §9.2).
    h.deliver_completions(1);

    // The library: every app, with its highest risk as the trailing text,
    // and the number of updates.
    assert_eq!(h.screen_title(), "Library");
    let badge = h.one(ControlKind::Badge, |_| true);
    assert_eq!(h.tree.get(badge).unwrap().str_prop(prop::LABEL), Some("1 update available"));
    assert!(h.texts().iter().any(|t| t == "1 update is available."), "{:?}", h.texts());
    let notes_row = h.row("Notes");
    assert!(h.tree.get(notes_row).unwrap().str_prop(prop::SUBTITLE).unwrap().ends_with("· Update available: 1.1.0"));
    assert_eq!(h.row_titles(), ["Notes", "Weather"]);
    let notes_row = h.row("Notes");
    assert_eq!(h.tree.get(notes_row).unwrap().str_prop(prop::TRAILING), Some("Medium risk"));

    // Group tabs and the text filter.
    let tabs = h.one(ControlKind::Tabs, |_| true);
    assert_eq!(h.tree.get(tabs).unwrap().str_prop(prop::ITEMS), Some("All\u{1f}Work"));
    h.fire(tabs, event::CHANGE, "Work".into());
    assert_eq!(h.row_titles(), ["Notes"]);
    h.fire(tabs, event::CHANGE, "All".into());
    let filter = h.labeled(ControlKind::TextField, "Find in library");
    h.tree.set_local_prop(filter, prop::VALUE, "wea".into());
    h.fire(filter, event::CHANGE, "wea".into());
    assert_eq!(h.row_titles(), ["Weather"]);
    h.fire(filter, event::CHANGE, "".into());

    // The app page: publisher, the capability label, "cannot" text.
    let notes_row = h.row("Notes");
    h.fire(notes_row, event::PRESS, Value::Null);
    assert_eq!(h.screen_title(), "Notes");
    assert!(h.texts().iter().any(|t| t == "Signed by Example (ed25519:abc)"), "{:?}", h.texts());
    let clip = h.row("Read the clipboard");
    let subtitle = h.tree.get(clip).unwrap().str_prop(prop::SUBTITLE).unwrap().to_owned();
    assert!(subtitle.starts_with("Medium risk · clipboard.read · Not decided yet"), "{subtitle}");
    let label = h.one(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("What this app can do"));
    assert_eq!(h.tree.get(label).unwrap().str_prop(prop::FOOTER), Some("This app cannot use the network, read your files."));

    // Change a grant with its toggle.
    let toggle = h.labeled(ControlKind::Toggle, "Allow clipboard.read");
    h.fire(toggle, event::CHANGE, Value::Bool(true));
    assert!(lib.lock().unwrap().apps[0].caps[1].4, "the grant reached the backend");
    let clip = h.row("Read the clipboard");
    let subtitle = h.tree.get(clip).unwrap().str_prop(prop::SUBTITLE).unwrap().to_owned();
    assert!(subtitle.contains("· Allowed"), "{subtitle}");

    // Update: the app page shows it with the capability that it adds.
    assert!(
        h.texts().iter().any(|t| t.starts_with("Version 1.1.0 is available from main. It also asks for: net:sync.example.com.")),
        "{:?}",
        h.texts()
    );
    h.fire(h.labeled(ControlKind::Button, "Update to 1.1.0"), event::PRESS, Value::Null);
    h.deliver_completions(1);
    assert!(h.texts().iter().any(|t| t == "Version 1.1.0 is installed."), "{:?}", h.texts());
    assert!(h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Update to 1.1.0")).is_empty());
    // The new capability is not decided yet: the Hub asks at the next open.
    let sync = h.row("Connect to sync.example.com");
    let subtitle = h.tree.get(sync).unwrap().str_prop(prop::SUBTITLE).unwrap().to_owned();
    assert!(subtitle.starts_with("Medium risk · net:sync.example.com · Not decided yet"), "{subtitle}");

    // Versions: pin the old one, then run the newest again.
    let v1 = h.row("Version 1.0.0");
    assert_eq!(h.tree.get(v1).unwrap().str_prop(prop::SUBTITLE), Some("Installed · no capabilities"));
    assert_eq!(h.tree.get(h.row("Version 1.1.0")).unwrap().str_prop(prop::SUBTITLE), Some("Runs now · no capabilities"));
    h.fire(v1, event::PRESS, Value::Null);
    assert_eq!(lib.lock().unwrap().apps[0].pinned, "1.0.0");
    assert_eq!(h.tree.get(h.row("Version 1.0.0")).unwrap().str_prop(prop::TRAILING), Some("Pinned"));
    assert!(h.texts().iter().any(|t| t == "Version 1.0.0 · From the source main · com.example.notes"), "{:?}", h.texts());
    h.fire(h.labeled(ControlKind::Button, "Run the newest version"), event::PRESS, Value::Null);
    assert_eq!(lib.lock().unwrap().apps[0].pinned, "");
    assert!(h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Run the newest version")).is_empty());

    // Block the publisher: Open is disabled; unblock it again.
    h.fire(h.labeled(ControlKind::Button, "Block publisher"), event::PRESS, Value::Null);
    assert!(lib.lock().unwrap().apps.iter().all(|a| a.publisher_blocked));
    assert!(h.tree.get(h.labeled(ControlKind::Button, "Open")).unwrap().bool_prop(prop::DISABLED));
    h.fire(h.labeled(ControlKind::Button, "Unblock publisher"), event::PRESS, Value::Null);
    assert!(!lib.lock().unwrap().apps[0].publisher_blocked);
    assert!(!h.tree.get(h.labeled(ControlKind::Button, "Open")).unwrap().bool_prop(prop::DISABLED));

    // Launch: the host drains the request (`take_hub_launches`).
    h.fire(h.labeled(ControlKind::Button, "Open"), event::PRESS, Value::Null);
    assert_eq!(h.guest.take_hub_launches(), vec!["com.example.notes".to_owned()]);

    // Groups: the Work checkbox is on; turn it off.
    let work = h.labeled(ControlKind::Checkbox, "Work");
    assert!(h.tree.get(work).unwrap().bool_prop(prop::VALUE));
    h.fire(work, event::CHANGE, Value::Bool(false));
    assert!(lib.lock().unwrap().apps[0].groups.is_empty());

    // Block: Open is disabled and a badge shows; Unblock reverses it.
    h.fire(h.labeled(ControlKind::Action, "Block"), event::PRESS, Value::Null);
    assert!(lib.lock().unwrap().apps[0].blocked);
    assert!(h.tree.get(h.labeled(ControlKind::Button, "Open")).unwrap().bool_prop(prop::DISABLED));
    assert_eq!(h.find(ControlKind::Badge, |_| true).len(), 1);
    h.fire(h.labeled(ControlKind::Action, "Unblock"), event::PRESS, Value::Null);
    assert!(!lib.lock().unwrap().apps[0].blocked);

    // Remove goes back to the library.
    h.fire(h.labeled(ControlKind::Action, "Remove from library"), event::PRESS, Value::Null);
    assert_eq!(h.screen_title(), "Library");
    assert_eq!(h.row_titles(), ["Weather"]);

    // A new group, from the sheet.
    h.fire(h.labeled(ControlKind::Action, "New group"), event::PRESS, Value::Null);
    let name = h.labeled(ControlKind::TextField, "Group name");
    h.tree.set_local_prop(name, prop::VALUE, "Family".into());
    h.fire(name, event::CHANGE, "Family".into());
    h.fire(h.labeled(ControlKind::Button, "Create group"), event::PRESS, Value::Null);
    assert_eq!(lib.lock().unwrap().groups, ["Work", "Family"]);
    let tabs = h.one(ControlKind::Tabs, |_| true);
    assert_eq!(h.tree.get(tabs).unwrap().str_prop(prop::ITEMS), Some("All\u{1f}Work\u{1f}Family"));
    // The new group is selected, and it is empty.
    assert_eq!(h.tree.get(tabs).unwrap().str_prop(prop::VALUE), Some("Family"));
    assert!(h.row_titles().is_empty());
    h.fire(tabs, event::CHANGE, "All".into());

    // Discover: search across sources, with the failing source shown.
    h.primary("Discover");
    let query = h.labeled(ControlKind::TextField, "Search sources");
    h.tree.set_local_prop(query, prop::VALUE, "ti".into());
    h.fire(query, event::CHANGE, "ti".into());
    h.fire(h.labeled(ControlKind::Button, "Search"), event::PRESS, Value::Null);
    h.deliver_completions(1);
    assert!(h.texts().iter().any(|t| t == "1 app found."), "{:?}", h.texts());
    assert_eq!(h.row_titles(), ["broken: cannot reach the source", "Timer"]);
    let timer_row = h.row("Timer");
    assert_eq!(h.tree.get(timer_row).unwrap().str_prop(prop::TRAILING), Some("Install"));

    // Install: the completion reloads the library.
    h.fire(timer_row, event::PRESS, Value::Null);
    h.deliver_completions(1);
    assert!(h.texts().iter().any(|t| t == "Timer is in your library."), "{:?}", h.texts());
    assert_eq!(h.tree.get(h.row("Timer")).unwrap().str_prop(prop::TRAILING), Some("In library"));
    h.primary("Library");
    assert_eq!(h.row_titles(), ["Weather", "Timer"]);
}

/// A guest that the host refused `hub.manage` still runs: it shows why the
/// library is empty and never traps (SPEC.md §8.5).
#[test]
fn the_hub_ui_without_a_backend_shows_the_denial() {
    let artifact = build_hub();
    let runner = Runner::new().unwrap();
    let mut guest = runner
        .load_with_policy_and_hub(
            &artifact.component,
            Limits::default(),
            Policy::new(["hub.manage".to_owned()]),
            Kv::in_memory(),
            Box::new(MemoryClipboard::default()),
            None,
        )
        .unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        tree.apply(&commit).unwrap();
    }
    let h = Harness { guest, tree, _runner: runner };
    assert!(h.texts().iter().any(|t| t == "The host refused to show the library (denied:unsupported)."), "{:?}", h.texts());
    assert!(h.row_titles().is_empty());
}
