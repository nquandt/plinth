//! `plinth:dialog` and the async host-call machinery it is built on
//! (SPEC.md §8.4, §8.5): a request id returned at once, a callback fired
//! later by a `completion` event, and a callback that survives a GC while
//! its request is pending (SPEC.md §5.4).

use plinth_compiler::driver::MemFs;
use plinth_protocol::{ControlKind, Event, NodeId, Value, Writer, event};
use plinth_runner_wasmtime::{Guest, Limits, Runner};
use plinth_ui::tree::{Node, Tree};

const SRC: &str = r#"import { app, Screen, Text, Button, signal } from "plinth:ui";
import { confirm } from "plinth:dialog";
const deleted = signal(false);
function Home() {
  return <Screen title="Home">
    <Text>{deleted() ? "gone" : "here"}</Text>
    <Button label="del" onPress={() => confirm("Delete?", (ok) => { if (ok) { deleted.set(true); } })} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

/// Compiles `SRC` and starts it. `gc_stress` turns on the collect-after-
/// every-event mode (SPEC.md §16) so the test also proves a pending
/// dialog request's callback survives a collection.
fn start(gc_stress: bool) -> (Guest, Tree) {
    let fs = MemFs::default().with("app/main.tsx", SRC);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let args =
        if gc_stress { plinth_protocol::init_arg::one(plinth_protocol::init_arg::GC_STRESS, &[]) } else { Vec::new() };
    let mut tree = Tree::new();
    for commit in guest.init(&args).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    (guest, tree)
}

fn find(tree: &Tree, kind: ControlKind, pred: impl Fn(&Node) -> bool) -> NodeId {
    let mut stack: Vec<NodeId> = tree.screens().map(|(_, id)| id).collect();
    while let Some(id) = stack.pop() {
        let node = tree.get(id).unwrap();
        if node.kind == Some(kind) && pred(node) {
            return id;
        }
        stack.extend(node.children.iter());
    }
    panic!("no matching {kind:?} found");
}

fn press(guest: &mut Guest, tree: &mut Tree, button: NodeId) {
    let handler = tree.get(button).and_then(|n| n.handler(event::PRESS)).expect("button has a handler");
    let mut w = Writer::new();
    w.event(&Event::Ui { handler, event: event::PRESS, value: Value::Null });
    for commit in guest.on_event(w.as_bytes()).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
}

fn answer(guest: &mut Guest, tree: &mut Tree, id: u32, result: Value) {
    for commit in guest.answer_dialog(id, result).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
}

fn text(tree: &Tree) -> String {
    let id = find(tree, ControlKind::Text, |_| true);
    tree.get(id).unwrap().text.clone().unwrap()
}

#[test]
fn confirm_opens_a_request_and_the_answer_reaches_the_callback() {
    let (mut guest, mut tree) = start(false);
    assert_eq!(text(&tree), "here");
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);

    let pending = guest.pending_dialogs().to_vec();
    assert_eq!(pending.len(), 1, "exactly one dialog request should be open");
    assert_eq!(pending[0].kind, plinth_runner_wasmtime::DialogKind::Confirm);
    assert_eq!(pending[0].message, "Delete?");
    let id = pending[0].id;

    answer(&mut guest, &mut tree, id, Value::Bool(true));
    assert_eq!(text(&tree), "gone");
    assert!(guest.pending_dialogs().is_empty());
}

#[test]
fn a_confirm_callback_survives_a_gc_between_the_call_and_its_answer() {
    let (mut guest, mut tree) = start(true);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    // `press` already triggered a collection (stress mode runs one after
    // every on-event): the pending request's callback must have survived
    // it (SPEC.md §5.4) for this to still work.
    let id = guest.pending_dialogs()[0].id;
    answer(&mut guest, &mut tree, id, Value::Bool(true));
    assert_eq!(text(&tree), "gone");
}

#[test]
fn an_unknown_request_id_is_ignored_not_an_error() {
    let (mut guest, mut tree) = start(false);
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);
    // Answering a request id nobody opened must not trap and must not
    // touch the tree.
    answer(&mut guest, &mut tree, 999_999, Value::Bool(true));
    assert_eq!(text(&tree), "here");
    // The real request is still open and answers normally afterward.
    let id = guest.pending_dialogs()[0].id;
    answer(&mut guest, &mut tree, id, Value::Bool(true));
    assert_eq!(text(&tree), "gone");
}

const ALERT_SRC: &str = r#"import { app, Screen, Text, Button } from "plinth:ui";
import { alert } from "plinth:dialog";
function Home() {
  return <Screen title="Home">
    <Text>hi</Text>
    <Button label="hi" onPress={() => alert("Hi!")} />
  </Screen>;
}
export default app({ screens: { home: { title: "Home", component: Home } } });
"#;

/// `alert`'s `done` is optional (SPEC.md §8.5): calling it with just a
/// message compiles and opens a request like the two-argument form.
#[test]
fn alert_with_no_done_callback_still_opens_a_request() {
    let fs = MemFs::default().with("app/main.tsx", ALERT_SRC);
    let (front, artifact) = plinth_compiler::compile(&fs).expect("compile");
    let artifact = artifact.unwrap_or_else(|| {
        let diags: Vec<String> = front.diags.iter().map(|d| front.sources.render(d)).collect();
        panic!("compile errors:\n{}", diags.join("\n"))
    });
    let runner = Runner::new().unwrap();
    let mut guest = runner.load(&artifact.component, Limits::default()).unwrap();
    let mut tree = Tree::new();
    for commit in guest.init(&[]).unwrap() {
        let errors = tree.apply(&commit).unwrap();
        assert!(errors.is_empty(), "op errors: {errors:?}");
    }
    let button = find(&tree, ControlKind::Button, |_| true);
    press(&mut guest, &mut tree, button);

    let pending = guest.pending_dialogs().to_vec();
    assert_eq!(pending.len(), 1, "exactly one dialog request should be open");
    assert_eq!(pending[0].kind, plinth_runner_wasmtime::DialogKind::Alert);
    assert_eq!(pending[0].message, "Hi!");
    let id = pending[0].id;

    // Answering with no `done` must not trap: the no-op closure just runs.
    answer(&mut guest, &mut tree, id, Value::Null);
    assert!(guest.pending_dialogs().is_empty());
}
