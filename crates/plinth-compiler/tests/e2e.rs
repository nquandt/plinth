//! Compiles the Plinth TS examples and runs them in wasmtime. They must
//! behave the same as the M0 hand-written guests (SPEC.md §15, M1 exit).

use plinth_compiler::driver::DiskFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event, prop};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The capability names declared in `<example>/plinth.toml`'s
/// `[[capabilities]]` tables (SPEC.md §11), read with a minimal parse so
/// this test crate does not need a `plinth-package` dev-dependency.
fn declared_capabilities(root: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(root.join("plinth.toml")) else { return Vec::new() };
    let Ok(value) = text.parse::<toml::Value>() else { return Vec::new() };
    value
        .get("capabilities")
        .and_then(|c| c.as_array())
        .map(|caps| caps.iter().filter_map(|c| c.get("name")?.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn build(name: &str) -> plinth_compiler::Artifact {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let caps = declared_capabilities(&root);
    let fs = DiskFs { root };
    let (front, artifact) = plinth_compiler::compile_with_capabilities(&fs, &caps).expect("compile");
    let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
    artifact.unwrap_or_else(|| panic!("{name} has errors:\n{}", diags.join("\n")))
}

struct Harness {
    guest: Guest,
    tree: Tree,
    _runner: Runner,
}

impl Harness {
    fn start(bytes: &[u8]) -> Self {
        let runner = Runner::new().unwrap();
        let mut guest = runner.load(bytes, Limits::default()).unwrap();
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

    fn fire(&mut self, node: NodeId, ev: u16, value: Value) -> Duration {
        let handler = self.tree.get(node).and_then(|n| n.handler(ev)).expect("node has a handler");
        let mut w = Writer::new();
        w.event(&Event::Ui { handler, event: ev, value });
        let start = Instant::now();
        let commits = self.guest.on_event(w.as_bytes());
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits.unwrap() {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
        start.elapsed()
    }

    /// Fires every timer due by `now` and applies what the guest commits.
    fn fire_timers(&mut self, now: Instant) {
        let commits = self.guest.fire_due_timers(now).unwrap();
        for log in self.guest.take_logs() {
            eprintln!("guest: {log}");
        }
        for commit in commits {
            let errors = self.tree.apply(&commit).unwrap();
            assert!(errors.is_empty(), "op errors: {errors:?}");
        }
    }

    fn find(&self, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        let mut found = Vec::new();
        let mut stack: Vec<NodeId> = self.tree.screens().map(|(_, id)| id).collect();
        stack.reverse();
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
        assert_eq!(f.len(), 1, "expected one {kind:?}, found {}", f.len());
        f[0]
    }

    fn label(&self, label: &str) -> NodeId {
        self.one(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some(label))
    }

    fn text_of(&self, id: NodeId) -> String {
        self.tree.get(id).unwrap().text.clone().unwrap_or_default()
    }

    fn row_titles(&self) -> Vec<String> {
        let list = self.one(ControlKind::List, |_| true);
        self.tree
            .get(list)
            .unwrap()
            .children
            .iter()
            .filter_map(|r| self.tree.get(*r))
            .filter(|n| n.kind == Some(ControlKind::Row))
            .map(|n| n.str_prop(prop::TITLE).unwrap_or_default().to_owned())
            .collect()
    }
}

#[test]
fn counter_behaves_like_m0() {
    let art = build("counter");
    eprintln!("counter: app {} B, core {} KiB, component {} KiB", art.app.len(), art.core_size / 1024, art.component.len() / 1024);
    // SPEC.md §5.5 size targets.
    // The size of the app artifacts is what counts (SPEC.md §5.5). The linker
    // stubs the runtime functions that an app does not reach, so the raw
    // runtime can grow without a cost to apps that do not use the new code.
    // The counter core module (runtime and app) must stay within 60 KiB. The
    // raw runtime limit is only a guard against accidents.
    assert!(art.core_size <= 60 * 1024, "the counter core module is over 60 KiB");
    assert!(plinth_compiler::link::runtime().len() <= 256 * 1024, "plinth-rt is over 256 KiB");
    assert!(art.component.len() <= 80 * 1024, "the counter artifact is over 80 KiB");
    let mut h = Harness::start(&art.component);
    let root = h.tree.current_root().unwrap();
    assert_eq!(root.str_prop(prop::TITLE), Some("Counter"));
    assert_eq!(root.str_prop(prop::ICON), Some("number"));
    let value = h.one(ControlKind::Heading, |_| true);
    let parity = h.one(ControlKind::Text, |_| true);
    assert_eq!(h.text_of(value), "0");
    assert_eq!(h.text_of(parity), "even");

    let inc = h.label("Increment");
    for _ in 0..3 {
        h.fire(inc, event::PRESS, Value::Null);
    }
    assert_eq!(h.text_of(value), "3");
    assert_eq!(h.text_of(parity), "odd");
    h.fire(h.label("Decrement"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(value), "2");
    h.fire(h.label("Reset"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(value), "0");

    // M1 exit: a round trip stays under 1 ms.
    let mut times: Vec<Duration> = (0..200).map(|_| h.fire(inc, event::PRESS, Value::Null)).collect();
    times.sort();
    eprintln!("round trip: median {:?}, max {:?}", times[100], times[199]);
    assert!(times[100] < Duration::from_millis(1));
    assert_eq!(h.text_of(value), "200");
}

#[test]
fn todo_behaves_like_m0() {
    let art = build("todo");
    eprintln!("todo: app {} B, core {} KiB, component {} KiB", art.app.len(), art.core_size / 1024, art.component.len() / 1024);
    let mut h = Harness::start(&art.component);
    assert_eq!(h.tree.screens().count(), 2);
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler"]);
    let summary = h.one(ControlKind::Text, |n| n.text.as_deref().is_some_and(|t| t.contains("remaining")));
    assert_eq!(h.text_of(summary), "3 of 3 remaining");

    // The Add button is disabled while the draft is empty.
    let add = h.label("Add task");
    assert!(h.tree.get(add).unwrap().bool_prop(prop::DISABLED));

    // Type, then submit with Enter. The guest clears the field.
    let field = h.one(ControlKind::TextField, |_| true);
    h.tree.set_local_prop(field, prop::VALUE, "Buy milk".into());
    h.fire(field, event::CHANGE, "Buy milk".into());
    assert!(!h.tree.get(add).unwrap().bool_prop(prop::DISABLED));
    h.fire(field, event::SUBMIT, Value::Null);
    assert_eq!(h.row_titles().last().map(String::as_str), Some("Buy milk"));
    assert_eq!(h.tree.get(field).unwrap().str_prop(prop::VALUE), Some(""));
    assert_eq!(h.text_of(summary), "4 of 4 remaining");

    // Complete the first task.
    let first_toggle = h.find(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Done"))[0];
    h.fire(first_toggle, event::CHANGE, Value::Bool(true));
    assert_eq!(h.text_of(summary), "3 of 4 remaining");
    let first_row = h.find(ControlKind::Row, |_| true)[0];
    assert_eq!(h.tree.get(first_row).unwrap().str_prop(prop::SUBTITLE), Some("Done"));

    // Settings: hide completed tasks, then show them again.
    let show = h.one(ControlKind::Toggle, |n| n.str_prop(prop::LABEL) == Some("Show completed tasks"));
    h.fire(show, event::CHANGE, Value::Bool(false));
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);
    h.fire(show, event::CHANGE, Value::Bool(true));
    assert_eq!(h.row_titles(), ["Read SPEC.md", "Build the M0 host", "Write the compiler", "Buy milk"]);

    // The clear button shows the count, and clears.
    let clear = h.one(ControlKind::Button, |n| n.str_prop(prop::LABEL).is_some_and(|l| l.starts_with("Clear")));
    assert_eq!(h.tree.get(clear).unwrap().str_prop(prop::LABEL), Some("Clear 1 completed task"));
    h.fire(clear, event::PRESS, Value::Null);
    assert_eq!(h.row_titles(), ["Build the M0 host", "Write the compiler", "Buy milk"]);
    assert!(h.tree.get(clear).unwrap().bool_prop(prop::DISABLED));

    // Delete everything: the Empty control appears.
    while let Some(&d) = h.find(ControlKind::Button, |n| n.str_prop(prop::LABEL) == Some("Delete")).first() {
        h.fire(d, event::PRESS, Value::Null);
    }
    assert!(h.row_titles().is_empty());
    let empty = h.one(ControlKind::Empty, |_| true);
    assert_eq!(h.tree.get(empty).unwrap().str_prop(prop::TITLE), Some("No tasks yet"));

    // navigate() selects a primary screen.
    h.fire(h.label("Back to tasks"), event::PRESS, Value::Null);
    assert_eq!(h.tree.current_screen, 0);
}

#[test]
fn calculator_computes() {
    let art = build("calculator");
    let mut h = Harness::start(&art.component);
    let display = h.one(ControlKind::Heading, |_| true);
    let press = |h: &mut Harness, label: &str| {
        let b = h.label(label);
        h.fire(b, event::PRESS, Value::Null);
    };
    for k in ["1", "2", "+", "3", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "15");
    // An exhaustive switch (no default) and division by zero.
    for k in ["÷", "0", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "Error");
    press(&mut h, "C");
    for k in ["7", ".", "5", "×", "2", "="] {
        press(&mut h, k);
    }
    assert_eq!(h.text_of(display), "15");
    assert_eq!(h.row_titles().len(), 3, "history has one row per calculation");
}

#[test]
fn notes_add_select_and_search() {
    let art = build("notes");
    let mut h = Harness::start(&art.component);
    let before = h.row_titles().len();
    // Add a note.
    let fields = h.find(ControlKind::TextField, |n| n.str_prop(prop::PLACEHOLDER) == Some("Note title"));
    let title = fields[0];
    h.tree.set_local_prop(title, prop::VALUE, "Groceries".into());
    h.fire(title, event::CHANGE, "Groceries".into());
    h.fire(h.label("Add note"), event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before + 1);
    // Selecting a note shows the edit section (a region that reads a signal).
    assert!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).is_empty());
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    h.fire(row, event::PRESS, Value::Null);
    assert_eq!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).len(), 1);
    // Deleting the selected note (`selected() === id` on `number | null`) hides it again.
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Groceries"))[0];
    let delete = h.tree.get(row).unwrap().children[0];
    h.fire(delete, event::PRESS, Value::Null);
    assert_eq!(h.row_titles().len(), before);
    assert!(h.find(ControlKind::Section, |n| n.str_prop(prop::TITLE) == Some("Edit note")).is_empty());
    // Search filters the list.
    let search = h.one(ControlKind::TextField, |n| n.str_prop(prop::LABEL) == Some("Search"));
    h.tree.set_local_prop(search, prop::VALUE, "zzzz-no-match".into());
    h.fire(search, event::CHANGE, "zzzz-no-match".into());
    assert!(h.row_titles().is_empty());
}

#[test]
fn settings_gallery_checkbox_and_text_area() {
    let art = build("settings-gallery");
    let mut h = Harness::start(&art.component);

    let checkbox = h.one(ControlKind::Checkbox, |n| n.str_prop(prop::LABEL) == Some("Email notifications"));
    assert!(h.tree.get(checkbox).unwrap().bool_prop(prop::VALUE));
    let note = h.one(ControlKind::Text, |n| n.text.as_deref() == Some("You will get emails."));
    assert_eq!(h.text_of(note), "You will get emails.");

    // Toggling the checkbox flips its own signal (no `onChange` is given).
    // The host sets the displayed value at once, as a real click would (SPEC.md §8.4).
    h.tree.set_local_prop(checkbox, prop::VALUE, Value::Bool(false));
    h.fire(checkbox, event::CHANGE, Value::Bool(false));
    assert!(!h.tree.get(checkbox).unwrap().bool_prop(prop::VALUE));
    let note = h.one(ControlKind::Text, |_| true);
    assert_eq!(h.text_of(note), "Emails are off.");

    // The text area is a two-way `value` binding, like TextField.
    let bio = h.one(ControlKind::TextArea, |_| true);
    assert_eq!(h.tree.get(bio).unwrap().str_prop(prop::PLACEHOLDER), Some("Tell us about yourself"));
    h.tree.set_local_prop(bio, prop::VALUE, "Loves Rust".into());
    h.fire(bio, event::CHANGE, "Loves Rust".into());
    assert_eq!(h.tree.get(bio).unwrap().str_prop(prop::VALUE), Some("Loves Rust"));
}

#[test]
fn settings_gallery_slider_number_picker_progress_badge() {
    use plinth_protocol::{prop::MAX, prop::MIN, prop::OPTIONS, prop::STEP};

    let art = build("settings-gallery");
    let mut h = Harness::start(&art.component);

    // The slider is a two-way `Signal<number>` binding (SPEC.md §8.4).
    let volume = h.one(ControlKind::Slider, |n| n.str_prop(prop::LABEL) == Some("Volume"));
    let node = h.tree.get(volume).unwrap();
    assert_eq!(node.num_prop(prop::VALUE), Some(40.0));
    assert_eq!(node.num_prop(MIN), Some(0.0));
    assert_eq!(node.num_prop(MAX), Some(100.0));
    assert_eq!(node.num_prop(STEP), Some(5.0));
    h.tree.set_local_prop(volume, prop::VALUE, Value::Number(65.0));
    h.fire(volume, event::CHANGE, Value::Number(65.0));
    assert_eq!(h.tree.get(volume).unwrap().num_prop(prop::VALUE), Some(65.0));
    // The progress bar reads the same signal through a plain (non-binding) prop.
    let progress = h.one(ControlKind::Progress, |n| n.str_prop(prop::LABEL) == Some("Sync progress"));
    assert_eq!(h.tree.get(progress).unwrap().num_prop(prop::VALUE), Some(0.65));

    // The number field is a two-way `Signal<number>` binding too.
    let quantity = h.one(ControlKind::NumberField, |n| n.str_prop(prop::LABEL) == Some("Items"));
    assert_eq!(h.tree.get(quantity).unwrap().num_prop(prop::VALUE), Some(3.0));
    h.tree.set_local_prop(quantity, prop::VALUE, Value::Number(7.0));
    h.fire(quantity, event::CHANGE, Value::Number(7.0));
    assert_eq!(h.tree.get(quantity).unwrap().num_prop(prop::VALUE), Some(7.0));

    // The picker's `options` is a compile-time-joined string.
    let theme = h.one(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("Theme"));
    assert_eq!(h.tree.get(theme).unwrap().str_prop(OPTIONS), Some("system\u{1f}light\u{1f}dark"));
    assert_eq!(h.tree.get(theme).unwrap().str_prop(prop::VALUE), Some("system"));
    h.tree.set_local_prop(theme, prop::VALUE, "dark".into());
    h.fire(theme, event::CHANGE, "dark".into());
    assert_eq!(h.tree.get(theme).unwrap().str_prop(prop::VALUE), Some("dark"));
    // Picking a new plan updates the badge through its own `onChange`-free binding.
    let plan = h.one(ControlKind::Picker, |n| n.str_prop(prop::LABEL) == Some("Plan"));
    h.tree.set_local_prop(plan, prop::VALUE, "pro".into());
    h.fire(plan, event::CHANGE, "pro".into());
    let badge = h.one(ControlKind::Badge, |_| true);
    assert_eq!(h.tree.get(badge).unwrap().str_prop(prop::LABEL), Some("pro"));

    // Indeterminate progress has no `value` prop at all.
    let working = h.one(ControlKind::Progress, |n| n.str_prop(prop::LABEL) == Some("Working"));
    assert_eq!(h.tree.get(working).unwrap().num_prop(prop::VALUE), None);
}

// -- UI API 1.2: structure and stack navigation -----------------------------

#[test]
fn contacts_push_back_tabs_sheet_and_destructive_action() {
    let art = build("contacts");
    eprintln!("contacts: core {} KiB, component {} KiB", art.core_size / 1024, art.component.len() / 1024);
    let mut h = Harness::start(&art.component);

    // "list" is the only primary screen; "detail" is pushable only.
    assert_eq!(h.tree.screens().count(), 2);
    assert_eq!(h.tree.primary_screens().count(), 1);
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Contacts"));
    assert!(!h.tree.can_go_back());

    // Tabs: switching to "favorites" filters the list.
    assert_eq!(h.row_titles(), ["Ada Lovelace", "Grace Hopper", "Alan Turing"]);
    let tabs = h.one(ControlKind::Tabs, |_| true);
    assert_eq!(h.tree.get(tabs).unwrap().str_prop(prop::ITEMS), Some("all\u{1f}favorites"));
    h.fire(tabs, event::CHANGE, "favorites".into());
    assert_eq!(h.row_titles(), ["Ada Lovelace", "Grace Hopper"]);
    h.fire(tabs, event::CHANGE, "all".into());

    // Selecting a row pushes the detail screen (`navigate.push`).
    let row = h.find(ControlKind::Row, |n| n.str_prop(prop::TITLE) == Some("Ada Lovelace"))[0];
    h.fire(row, event::PRESS, Value::Null);
    assert!(h.tree.can_go_back());
    let detail = h.tree.current_root().unwrap();
    assert_eq!(detail.str_prop(prop::TITLE), Some("Ada Lovelace"));

    // Screen actions: Edit opens the Sheet; Save edits and closes it.
    let edit = h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("Edit"));
    h.fire(edit, event::PRESS, Value::Null);
    let sheet = h.one(ControlKind::Sheet, |_| true);
    assert!(h.tree.get(sheet).unwrap().bool_prop(prop::VALUE));
    let name_field = h.tree.get(sheet).unwrap().children[0];
    assert_eq!(h.tree.get(name_field).unwrap().str_prop(prop::LABEL), Some("Name"));
    h.tree.set_local_prop(name_field, prop::VALUE, "Ada, Countess Lovelace".into());
    h.fire(name_field, event::CHANGE, "Ada, Countess Lovelace".into());
    h.fire(h.label("Save"), event::PRESS, Value::Null);
    assert!(!h.tree.get(sheet).unwrap().bool_prop(prop::VALUE));
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Ada, Countess Lovelace"));

    // The destructive Delete action: the host shows a confirmation dialog
    // (SPEC.md §6.4) before firing `onPress`; at the protocol level, firing
    // the handler deletes the contact and `navigate.back()` pops the stack.
    let delete = h.one(ControlKind::Action, |n| n.str_prop(prop::LABEL) == Some("Delete"));
    assert_eq!(h.tree.get(delete).unwrap().enum_prop(prop::ROLE), plinth_protocol::button_role::DESTRUCTIVE);
    h.fire(delete, event::PRESS, Value::Null);
    assert!(!h.tree.can_go_back());
    assert_eq!(h.tree.current_root().unwrap().str_prop(prop::TITLE), Some("Contacts"));
    assert_eq!(h.row_titles(), ["Grace Hopper", "Alan Turing"]);
}

/// `plinth:time`'s setInterval/clearInterval (SPEC.md §8.5), needing no
/// capability.
#[test]
fn timer_starts_ticks_and_stops() {
    let art = build("timer");
    let mut h = Harness::start(&art.component);
    let heading = h.one(ControlKind::Heading, |_| true);
    assert_eq!(h.text_of(heading), "0.0s");
    assert!(h.guest.next_timer_deadline().is_none(), "no timer until Start is pressed");

    h.fire(h.label("Start"), event::PRESS, Value::Null);
    assert!(h.guest.next_timer_deadline().is_some());

    let t0 = Instant::now();
    h.fire_timers(t0 + Duration::from_millis(100));
    assert_eq!(h.text_of(heading), "0.1s");
    // A `TimerQueue` delivers one firing per poll (even when the poll is
    // late) and reschedules from its own due time, not from `now`.
    h.fire_timers(t0 + Duration::from_millis(250));
    assert_eq!(h.text_of(heading), "0.2s");

    h.fire(h.label("Stop"), event::PRESS, Value::Null);
    assert!(h.guest.next_timer_deadline().is_none(), "clearInterval cancels the pending timer");
    h.fire_timers(t0 + Duration::from_secs(5));
    assert_eq!(h.text_of(heading), "0.2s", "stopped: no more ticks");

    h.fire(h.label("Reset"), event::PRESS, Value::Null);
    assert_eq!(h.text_of(heading), "0.0s");
}
