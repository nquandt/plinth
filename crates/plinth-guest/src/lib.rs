//! A small retained-mode guest SDK for hand-written Plinth apps in Rust.
//!
//! M0 uses it to test the protocol and the host before the compiler exists
//! (SPEC.md §15). It writes ops into a buffer, keeps event handlers, and
//! commits one buffer per event. The keyed reconciler in [`Ui::reconcile`]
//! is the model for the one in `plinth-rt`.
//!
//! ```ignore
//! plinth_guest::app!(|ui: &mut plinth_guest::Ui| {
//!     let screen = ui.screen("Hello", "house");
//!     ui.text(screen, "Hello, world");
//!     ui.set_root(0, screen);
//! });
//! ```

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;

pub use plinth_protocol::{ControlKind, NodeId, Value, button_role, event, nav_kind, prop, text_style, tone};
use plinth_protocol::{Event, Op, Writer, decode_events};

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        path: "../../wit/plinth",
        world: "app",
        pub_export_macro: true,
        default_bindings_module: "plinth_guest::bindings",
    });
}

/// Defines the app entry points. The argument is a function that builds the
/// initial tree.
#[macro_export]
macro_rules! app {
    ($init:expr) => {
        struct __PlinthApp;
        impl $crate::bindings::Guest for __PlinthApp {
            fn init(args: ::std::vec::Vec<u8>) {
                $crate::runtime_init(&args, $init);
            }
            fn on_event(ev: ::std::vec::Vec<u8>) {
                $crate::runtime_on_event(&ev);
            }
        }
        $crate::bindings::export!(__PlinthApp with_types_in $crate::bindings);
    };
}

type Handler = Box<dyn FnMut(&mut Ui, Value)>;

/// The guest view of the tree, and the pending op buffer.
pub struct Ui {
    ops: Writer,
    next_id: NodeId,
    free_ids: Vec<NodeId>,
    parent: HashMap<NodeId, NodeId>,
    children: HashMap<NodeId, Vec<NodeId>>,
    next_handler: u32,
    handlers: HashMap<u32, Handler>,
    /// `(event, handler)` pairs per node.
    listeners: HashMap<NodeId, Vec<(u16, u32)>>,
    /// Handlers that were dropped while they ran.
    dropped_handlers: HashSet<u32>,
}

impl Ui {
    fn new() -> Self {
        Self {
            ops: Writer::new(),
            next_id: 1,
            free_ids: Vec::new(),
            parent: HashMap::new(),
            children: HashMap::new(),
            next_handler: 1,
            handlers: HashMap::new(),
            listeners: HashMap::new(),
            dropped_handlers: HashSet::new(),
        }
    }

    // -- Raw ops --------------------------------------------------------

    pub fn create(&mut self, kind: ControlKind) -> NodeId {
        let id = self.free_ids.pop().unwrap_or_else(|| {
            let id = self.next_id;
            self.next_id += 1;
            id
        });
        self.children.insert(id, Vec::new());
        self.ops.op(&Op::Create { id, kind: kind as u16 });
        id
    }

    /// Inserts `id` into `parent` before `before` (`0` appends). If `id` has
    /// a parent, this is a move.
    pub fn insert(&mut self, parent: NodeId, id: NodeId, before: NodeId) {
        let moved = self.detach(id);
        let siblings = self.children.entry(parent).or_default();
        let at = match before {
            0 => siblings.len(),
            b => siblings.iter().position(|c| *c == b).unwrap_or(siblings.len()),
        };
        siblings.insert(at, id);
        self.parent.insert(id, parent);
        let op = if moved { Op::Move { parent, id, before } } else { Op::Insert { parent, id, before } };
        self.ops.op(&op);
    }

    pub fn append(&mut self, parent: NodeId, id: NodeId) {
        self.insert(parent, id, 0);
    }

    /// Removes the node and its subtree, and drops their handlers.
    pub fn remove(&mut self, id: NodeId) {
        self.detach(id);
        self.ops.op(&Op::Remove { id });
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            if let Some(kids) = self.children.remove(&id) {
                stack.extend(kids);
            }
            self.parent.remove(&id);
            for (_, h) in self.listeners.remove(&id).unwrap_or_default() {
                self.drop_handler(h);
            }
            self.free_ids.push(id);
        }
    }

    pub fn set(&mut self, id: NodeId, prop: u16, value: impl Into<Value>) {
        self.ops.op(&Op::SetProp { id, prop, value: value.into() });
    }

    pub fn set_text(&mut self, id: NodeId, text: &str) {
        self.ops.op(&Op::Text { id, value: Value::from(text) });
    }

    /// Registers `f` for `event` on node `id`. It replaces an earlier handler
    /// for the same event.
    pub fn on(&mut self, id: NodeId, event: u16, f: impl FnMut(&mut Ui, Value) + 'static) {
        let handler = self.next_handler;
        self.next_handler += 1;
        self.handlers.insert(handler, Box::new(f));
        let list = self.listeners.entry(id).or_default();
        let old = list.iter().position(|(e, _)| *e == event).map(|pos| list.remove(pos).1);
        if let Some(old) = old {
            self.drop_handler(old);
        }
        self.listeners.entry(id).or_default().push((event, handler));
        self.ops.op(&Op::Listen { id, event, handler });
    }

    /// Makes `id` the root of primary screen `screen`.
    pub fn set_root(&mut self, screen: u32, id: NodeId) {
        self.ops.op(&Op::SetRoot { screen, id });
    }

    /// Selects a primary screen.
    pub fn select_screen(&mut self, screen: u32) {
        self.ops.op(&Op::Navigate { kind: nav_kind::SELECT_PRIMARY, screen, args: Value::Null });
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn detach(&mut self, id: NodeId) -> bool {
        match self.parent.remove(&id) {
            Some(p) => {
                if let Some(siblings) = self.children.get_mut(&p) {
                    siblings.retain(|c| *c != id);
                }
                true
            }
            None => false,
        }
    }

    fn drop_handler(&mut self, h: u32) {
        if self.handlers.remove(&h).is_none() {
            // The handler runs now. Do not put it back after it returns.
            self.dropped_handlers.insert(h);
        }
    }

    // -- Control helpers -------------------------------------------------

    fn child(&mut self, parent: NodeId, kind: ControlKind) -> NodeId {
        let id = self.create(kind);
        if parent != 0 {
            self.append(parent, id);
        }
        id
    }

    /// Makes a detached screen. Pass it to [`Ui::set_root`].
    pub fn screen(&mut self, title: &str, icon: &str) -> NodeId {
        let id = self.create(ControlKind::Screen);
        self.set(id, prop::TITLE, title);
        self.set(id, prop::ICON, icon);
        id
    }

    pub fn section(&mut self, parent: NodeId, title: Option<&str>) -> NodeId {
        let id = self.child(parent, ControlKind::Section);
        if let Some(title) = title {
            self.set(id, prop::TITLE, title);
        }
        id
    }

    pub fn text(&mut self, parent: NodeId, text: &str) -> NodeId {
        let id = self.child(parent, ControlKind::Text);
        self.set_text(id, text);
        id
    }

    pub fn heading(&mut self, parent: NodeId, level: i32, text: &str) -> NodeId {
        let id = self.child(parent, ControlKind::Heading);
        self.set(id, prop::LEVEL, level);
        self.set_text(id, text);
        id
    }

    pub fn button(&mut self, parent: NodeId, label: &str, role: u16, on_press: impl FnMut(&mut Ui) + 'static) -> NodeId {
        let id = self.child(parent, ControlKind::Button);
        self.set(id, prop::LABEL, label);
        if role != button_role::DEFAULT {
            self.set(id, prop::ROLE, Value::Enum(role));
        }
        let mut on_press = on_press;
        self.on(id, event::PRESS, move |ui, _| on_press(ui));
        id
    }

    /// A text field. The host keeps the displayed text, and `on_change` gets
    /// each new value (SPEC.md §8.4).
    pub fn text_field(
        &mut self,
        parent: NodeId,
        label: &str,
        placeholder: &str,
        on_change: impl FnMut(&mut Ui, String) + 'static,
    ) -> NodeId {
        let id = self.child(parent, ControlKind::TextField);
        self.set(id, prop::LABEL, label);
        self.set(id, prop::PLACEHOLDER, placeholder);
        self.set(id, prop::VALUE, "");
        let mut on_change = on_change;
        self.on(id, event::CHANGE, move |ui, v| on_change(ui, v.as_str().unwrap_or_default().to_owned()));
        id
    }

    pub fn toggle(&mut self, parent: NodeId, label: &str, value: bool, on_change: impl FnMut(&mut Ui, bool) + 'static) -> NodeId {
        let id = self.child(parent, ControlKind::Toggle);
        self.set(id, prop::LABEL, label);
        self.set(id, prop::VALUE, value);
        let mut on_change = on_change;
        self.on(id, event::CHANGE, move |ui, v| on_change(ui, v.as_bool().unwrap_or(false)));
        id
    }

    pub fn list(&mut self, parent: NodeId) -> NodeId {
        self.child(parent, ControlKind::List)
    }

    pub fn row(&mut self, parent: NodeId, title: &str) -> NodeId {
        let id = self.child(parent, ControlKind::Row);
        self.set(id, prop::TITLE, title);
        id
    }

    pub fn empty(&mut self, parent: NodeId, title: &str, message: &str) -> NodeId {
        let id = self.child(parent, ControlKind::Empty);
        self.set(id, prop::TITLE, title);
        self.set(id, prop::MESSAGE, message);
        id
    }

    /// Keyed reconciliation of the children of `parent` (SPEC.md §7.3).
    ///
    /// `rows` holds the `(key, node)` pairs from the previous call. Kept rows
    /// get `update`, new rows get `create`, and missing rows are removed.
    /// Then the children are put in the order of `items` with the minimum of
    /// `move` ops for simple edits.
    pub fn reconcile<K: Eq + Hash + Clone, T>(
        &mut self,
        parent: NodeId,
        rows: &mut Vec<(K, NodeId)>,
        items: &[T],
        key: impl Fn(&T) -> K,
        mut create: impl FnMut(&mut Ui, &T) -> NodeId,
        mut update: impl FnMut(&mut Ui, NodeId, &T),
    ) {
        let mut old: HashMap<K, NodeId> = rows.drain(..).collect();
        for item in items {
            let k = key(item);
            let node = match old.remove(&k) {
                Some(node) => {
                    update(self, node, item);
                    node
                }
                None => create(self, item),
            };
            rows.push((k, node));
        }
        for (_, node) in old {
            self.remove(node);
        }
        for (i, (_, node)) in rows.iter().enumerate() {
            let current = self.children(parent);
            if current.get(i) == Some(node) {
                continue;
            }
            let before = current.get(i).copied().unwrap_or(0);
            self.insert(parent, *node, before);
        }
    }

    fn flush(&mut self) {
        if !self.ops.is_empty() {
            bindings::plinth::app::ui::commit(&self.ops.take());
        }
    }

    fn dispatch(&mut self, ev: Event) {
        let Event::Ui { handler, value, .. } = ev else { return };
        let Some(mut f) = self.handlers.remove(&handler) else { return };
        f(self, value);
        if !self.dropped_handlers.remove(&handler) {
            self.handlers.insert(handler, f);
        }
    }
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

#[doc(hidden)]
pub fn runtime_init(_args: &[u8], build: impl FnOnce(&mut Ui)) {
    let mut ui = Ui::new();
    build(&mut ui);
    ui.flush();
    UI.with(|cell| *cell.borrow_mut() = Some(ui));
}

#[doc(hidden)]
pub fn runtime_on_event(buf: &[u8]) {
    // A malformed event buffer is a host bug. Trap, so the host shows it.
    let events = decode_events(buf).expect("malformed event buffer");
    let mut ui = UI.with(|cell| cell.borrow_mut().take()).expect("on-event before init");
    for ev in events {
        ui.dispatch(ev);
    }
    // One commit per event batch (SPEC.md §7.1).
    ui.flush();
    UI.with(|cell| *cell.borrow_mut() = Some(ui));
}
