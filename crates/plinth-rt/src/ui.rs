//! The guest side of the semantic tree: node ids, the op buffer, dynamic
//! child regions, event handlers, two-way bindings and the keyed list
//! reconciler (SPEC.md §7.2, §7.3, §8.4).

use crate::reactive::{self, Cleanup, EffectKind, ScopeId};
use crate::{Callable, Val, invoke, strings};
use plinth_protocol::{ControlKind, NodeId, Op, Value, Writer, event, nav_kind, prop};
use std::cell::RefCell;
use std::collections::HashMap;

/// A child position of a parent: a static node, or a dynamic region.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Node(NodeId),
    Region(u32),
}

struct Region {
    parent: NodeId,
    callable: Callable,
    node: NodeId,
    /// The scope of the current content.
    content: Option<ScopeId>,
}

enum Handler {
    User(Callable),
    Bind(u32),
}

#[derive(Clone, Copy, PartialEq)]
pub enum BindKind {
    Str,
    Bool,
}

struct Bind {
    node: NodeId,
    signal: u32,
    kind: BindKind,
    /// The last value that the host and the guest agree on.
    synced: Option<Val>,
}

#[derive(Clone, PartialEq)]
enum Key {
    Num(u64),
    Str(String),
}

struct Row {
    key: Key,
    item: Val,
    node: NodeId,
    scope: ScopeId,
}

struct List {
    node: NodeId,
    owner: ScopeId,
    items: Callable,
    key: Callable,
    row: Callable,
    empty: Option<Callable>,
    rows: Vec<Row>,
    empty_node: Option<(NodeId, ScopeId)>,
}

#[derive(Default)]
struct Ui {
    ops: Writer,
    next_node: NodeId,
    free_nodes: Vec<NodeId>,
    slots: HashMap<NodeId, Vec<Slot>>,
    regions: Vec<Option<Region>>,
    next_handler: u32,
    handlers: HashMap<u32, Handler>,
    binds: Vec<Option<Bind>>,
    lists: Vec<Option<List>>,
}

thread_local! {
    static UI: RefCell<Ui> = RefCell::new(Ui { next_node: 1, next_handler: 1, ..Default::default() });
}

fn with<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with_borrow_mut(f)
}

/// Returns the pending op buffer and leaves it empty.
pub fn take_ops() -> Vec<u8> {
    with(|u| u.ops.take())
}

fn val_to_value(v: Val) -> Value {
    match v {
        Val::None => Value::Null,
        Val::F64(n) => Value::Number(n),
        Val::I32(i) => Value::Int(i),
        Val::Ref(p) => Value::Str(strings::as_str(p).to_owned()),
    }
}

// ---------------------------------------------------------------------------
// Nodes and props

pub fn create(kind: u16) -> NodeId {
    if ControlKind::from_u16(kind).is_none() {
        crate::trap("unknown control kind");
    }
    let id = with(|u| {
        let id = u.free_nodes.pop().unwrap_or_else(|| {
            let id = u.next_node;
            u.next_node += 1;
            id
        });
        u.ops.op(&Op::Create { id, kind });
        id
    });
    reactive::add_cleanup(Cleanup::Node(id));
    id
}

pub fn set_prop(id: NodeId, prop: u16, value: Value) {
    with(|u| u.ops.op(&Op::SetProp { id, prop, value }));
}

pub fn set_text(id: NodeId, text: &str) {
    with(|u| u.ops.op(&Op::Text { id, value: Value::Str(text.to_owned()) }));
}

/// Appends a static child.
pub fn append(parent: NodeId, child: NodeId) {
    with(|u| {
        u.slots.entry(parent).or_default().push(Slot::Node(child));
        u.ops.op(&Op::Insert { parent, id: child, before: 0 });
    });
}

pub fn set_root(screen: u32, id: NodeId) {
    with(|u| u.ops.op(&Op::SetRoot { screen, id }));
}

pub fn navigate(screen: u32) {
    with(|u| u.ops.op(&Op::Navigate { kind: nav_kind::SELECT_PRIMARY, screen, args: Value::Null }));
}

/// The first node after slot `index` of `parent`, or 0 (append).
fn node_after(u: &Ui, parent: NodeId, index: usize) -> NodeId {
    let Some(slots) = u.slots.get(&parent) else { return 0 };
    for slot in &slots[index + 1..] {
        match *slot {
            Slot::Node(n) => return n,
            Slot::Region(r) => {
                let n = u.regions[r as usize].as_ref().map(|r| r.node).unwrap_or(0);
                if n != 0 {
                    return n;
                }
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Regions: a dynamic child, for example `{cond() && <X/>}`

pub fn region(parent: NodeId, callable: Callable) {
    let id = with(|u| {
        let id = u.regions.len() as u32;
        u.regions.push(Some(Region { parent, callable, node: 0, content: None }));
        u.slots.entry(parent).or_default().push(Slot::Region(id));
        id
    });
    reactive::add_cleanup(Cleanup::Region(id));
    reactive::effect_new(EffectKind::Region(id));
}

fn run_region(id: u32) {
    let Some((callable, old_node, old_content)) = with(|u| {
        u.regions[id as usize].as_mut().map(|r| (r.callable, r.node, r.content.take()))
    }) else {
        return;
    };
    // The content gets its own scope, which the next run disposes. The
    // region effect tracks the reads of the callable (the condition).
    let scope = reactive::new_scope(reactive::current_scope());
    let node = reactive::with_scope(scope, || match invoke(callable, Val::None) {
        Val::I32(n) => n as NodeId,
        _ => 0,
    });
    if old_node != 0 && old_node != node {
        with(|u| u.ops.op(&Op::Remove { id: old_node }));
    }
    if let Some(c) = old_content {
        reactive::dispose(c);
    }
    with(|u| {
        let parent = u.regions[id as usize].as_ref().map(|r| r.parent).unwrap_or(0);
        if node != 0 && node != old_node {
            let index = u.slots[&parent].iter().position(|s| *s == Slot::Region(id)).unwrap_or(0);
            let before = node_after(u, parent, index);
            u.ops.op(&Op::Insert { parent, id: node, before });
        }
        if let Some(r) = u.regions[id as usize].as_mut() {
            r.node = node;
            r.content = Some(scope);
        }
    });
}

// ---------------------------------------------------------------------------
// Handlers and bindings

pub fn listen(node: NodeId, ev: u16, callable: Callable) {
    let h = with(|u| {
        let h = u.next_handler;
        u.next_handler += 1;
        u.handlers.insert(h, Handler::User(callable));
        u.ops.op(&Op::Listen { id: node, event: ev, handler: h });
        h
    });
    reactive::add_cleanup(Cleanup::Handler(h));
}

/// A two-way binding of `value` to a signal (SPEC.md §8.4).
pub fn bind(node: NodeId, signal: u32, kind: BindKind) {
    let id = with(|u| {
        let id = u.binds.len() as u32;
        u.binds.push(Some(Bind { node, signal, kind, synced: None }));
        let h = u.next_handler;
        u.next_handler += 1;
        u.handlers.insert(h, Handler::Bind(id));
        u.ops.op(&Op::Listen { id: node, event: event::CHANGE, handler: h });
        id
    });
    reactive::add_cleanup(Cleanup::Bind(id));
    reactive::effect_new(EffectKind::Bind(id));
}

fn run_bind(id: u32) {
    let Some((node, signal, kind, synced)) =
        with(|u| u.binds[id as usize].as_ref().map(|b| (b.node, b.signal, b.kind, b.synced)))
    else {
        return;
    };
    let v = reactive::signal_get(signal);
    let same = match (synced, v) {
        (Some(Val::Ref(a)), Val::Ref(b)) => strings::as_str(a) == strings::as_str(b),
        (Some(a), b) => a.same(&b),
        (None, _) => false,
    };
    if same {
        return; // The host shows this value already.
    }
    let value = match (kind, v) {
        (BindKind::Bool, Val::I32(b)) => Value::Bool(b != 0),
        (_, v) => val_to_value(v),
    };
    with(|u| {
        u.ops.op(&Op::SetProp { id: node, prop: prop::VALUE, value });
        if let Some(b) = u.binds[id as usize].as_mut() {
            b.synced = Some(v);
        }
    });
}

/// Converts a host event value to a guest value.
pub fn value_to_val(v: &Value) -> Val {
    match v {
        Value::Null => Val::None,
        Value::Bool(b) => Val::I32(*b as i32),
        Value::Int(i) => Val::I32(*i),
        Value::Number(n) => Val::F64(*n),
        Value::Enum(e) => Val::I32(*e as i32),
        Value::Str(s) => Val::Ref(strings::from_str(s)),
        Value::Handle(h) => Val::I32(*h as i32),
        Value::List(_) => Val::None,
    }
}

/// Runs the handler for one `ui` event.
pub fn dispatch(handler: u32, value: &Value) {
    enum Action {
        User(Callable),
        Bind(u32, BindKind),
    }
    let action = with(|u| match u.handlers.get(&handler) {
        Some(Handler::User(c)) => Some(Action::User(*c)),
        Some(Handler::Bind(b)) => u.binds[*b as usize].as_ref().map(|bind| Action::Bind(*b, bind.kind)),
        None => None,
    });
    match action {
        // A handler for a node that was removed in the same batch.
        None => {}
        Some(Action::User(c)) => {
            reactive::untracked(|| invoke(c, value_to_val(value)));
        }
        Some(Action::Bind(b, kind)) => {
            let v = match (kind, value) {
                (BindKind::Bool, Value::Bool(x)) => Val::I32(*x as i32),
                (BindKind::Str, Value::Str(_)) => value_to_val(value),
                _ => return,
            };
            let signal = with(|u| {
                let bind = u.binds[b as usize].as_mut()?;
                bind.synced = Some(v);
                Some(bind.signal)
            });
            if let Some(signal) = signal {
                reactive::signal_set(signal, v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Keyed lists (SPEC.md §7.3)

pub fn list(node: NodeId, items: Callable, key: Callable, row: Callable, empty: Option<Callable>) {
    let owner = reactive::current_scope();
    let id = with(|u| {
        let id = u.lists.len() as u32;
        u.lists.push(Some(List { node, owner, items, key, row, empty, rows: Vec::new(), empty_node: None }));
        id
    });
    reactive::add_cleanup(Cleanup::List(id));
    reactive::effect_new(EffectKind::List(id));
}

fn to_key(v: Val) -> Key {
    match v {
        Val::F64(n) => Key::Num(n.to_bits()),
        Val::I32(i) => Key::Num((i as f64).to_bits()),
        Val::Ref(p) => Key::Str(strings::as_str(p).to_owned()),
        Val::None => Key::Num(0),
    }
}

fn run_list(id: u32) {
    let Some((node, owner, items_c, key_c, row_c, empty_c)) =
        with(|u| u.lists[id as usize].as_ref().map(|l| (l.node, l.owner, l.items, l.key, l.row, l.empty)))
    else {
        return;
    };
    let arr = match invoke(items_c, Val::None) {
        Val::Ref(p) => p,
        _ => 0,
    };
    let items: Vec<Val> = if arr == 0 {
        Vec::new()
    } else {
        let n = crate::arrays::len(arr);
        match crate::gc::type_of(arr) {
            crate::gc::T_ARR_F64 => (0..n).map(|i| Val::F64(crate::arrays::get_f64(arr, i as i32))).collect(),
            crate::gc::T_ARR_REF => (0..n).map(|i| Val::Ref(crate::arrays::get_i32(arr, i as i32) as u32)).collect(),
            _ => (0..n).map(|i| Val::I32(crate::arrays::get_i32(arr, i as i32))).collect(),
        }
    };

    let mut old: Vec<Option<Row>> =
        with(|u| u.lists[id as usize].as_mut().map(|l| std::mem::take(&mut l.rows))).unwrap_or_default().into_iter().map(Some).collect();
    let mut rows = Vec::with_capacity(items.len());
    // The nodes of reused rows. A new row can get the id of a removed row,
    // so the order below must not match on ids alone.
    let mut kept: Vec<NodeId> = Vec::new();
    reactive::untracked(|| {
        for item in items {
            let key = to_key(invoke(key_c, item));
            let reuse = old.iter_mut().position(|r| r.as_ref().is_some_and(|r| r.key == key));
            match reuse.and_then(|i| old[i].take()) {
                // The same key and the same item: keep the row.
                Some(r) if r.item.same(&item) => {
                    kept.push(r.node);
                    rows.push(r)
                }
                stale => {
                    if let Some(r) = stale {
                        remove_row(r);
                    }
                    let scope = reactive::new_scope(owner);
                    let row_node = reactive::run_in_scope(scope, || match invoke(row_c, item) {
                        Val::I32(n) => n as NodeId,
                        _ => crate::trap("a List row must return an element"),
                    });
                    rows.push(Row { key, item, node: row_node, scope });
                }
            }
        }
    });
    for r in old.into_iter().flatten() {
        remove_row(r);
    }

    // Put the row nodes in order with insert and move ops.
    let mut current: Vec<NodeId> = ORDER
        .with_borrow_mut(|o| o.remove(&id))
        .unwrap_or_default()
        .into_iter()
        .filter(|n| kept.contains(n))
        .collect();
    with(|u| {
        for (i, r) in rows.iter().enumerate() {
            if current.get(i) == Some(&r.node) {
                continue;
            }
            let before = current.get(i).copied().unwrap_or(0);
            let existed = current.iter().position(|n| *n == r.node);
            let op = match existed {
                Some(_) => Op::Move { parent: node, id: r.node, before },
                None => Op::Insert { parent: node, id: r.node, before },
            };
            u.ops.op(&op);
            if let Some(pos) = existed {
                current.remove(pos);
            }
            current.insert(i, r.node);
        }
    });
    ORDER.with_borrow_mut(|o| o.insert(id, current));

    // The empty placeholder.
    let has_rows = !rows.is_empty();
    let empty_node = with(|u| {
        let l = u.lists[id as usize].as_mut().unwrap();
        l.rows = rows;
        l.empty_node
    });
    match (has_rows, empty_node, empty_c) {
        (true, Some((n, scope)), _) => {
            with(|u| {
                u.ops.op(&Op::Remove { id: n });
                u.lists[id as usize].as_mut().unwrap().empty_node = None;
            });
            reactive::dispose(scope);
        }
        (false, None, Some(c)) => {
            let scope = reactive::new_scope(owner);
            let n = reactive::run_in_scope(scope, || match invoke(c, Val::None) {
                Val::I32(n) => n as NodeId,
                _ => 0,
            });
            with(|u| {
                if n != 0 {
                    u.ops.op(&Op::Insert { parent: node, id: n, before: 0 });
                }
                u.lists[id as usize].as_mut().unwrap().empty_node = Some((n, scope));
            });
        }
        _ => {}
    }
}

thread_local! {
    /// The current child order of each list.
    static ORDER: RefCell<HashMap<u32, Vec<NodeId>>> = RefCell::new(HashMap::new());
}

fn remove_row(r: Row) {
    with(|u| u.ops.op(&Op::Remove { id: r.node }));
    reactive::dispose(r.scope);
}

// ---------------------------------------------------------------------------
// Effects and cleanup dispatch from the reactive core

pub fn run_effect(kind: EffectKind) {
    match kind {
        EffectKind::User(c) => {
            invoke(c, Val::None);
        }
        EffectKind::Region(id) => run_region(id),
        EffectKind::List(id) => run_list(id),
        EffectKind::Bind(id) => run_bind(id),
    }
}

pub fn cleanup(c: Cleanup) {
    match c {
        Cleanup::Handler(h) => {
            with(|u| u.handlers.remove(&h));
        }
        Cleanup::Node(n) => with(|u| {
            u.slots.remove(&n);
            u.free_nodes.push(n);
        }),
        Cleanup::Region(r) => {
            let content = with(|u| u.regions[r as usize].take().and_then(|r| r.content));
            if let Some(c) = content {
                reactive::dispose(c);
            }
        }
        Cleanup::Bind(b) => with(|u| {
            u.binds[b as usize] = None;
        }),
        Cleanup::List(l) => {
            let list = with(|u| u.lists[l as usize].take());
            ORDER.with_borrow_mut(|o| o.remove(&l));
            if let Some(list) = list {
                for r in list.rows {
                    reactive::dispose(r.scope);
                }
                if let Some((_, scope)) = list.empty_node {
                    reactive::dispose(scope);
                }
            }
        }
    }
}

/// Gives every heap reference that the UI state holds.
pub fn trace_roots(f: &mut dyn FnMut(u32)) {
    with(|u| {
        for h in u.handlers.values() {
            if let Handler::User(c) = h {
                f(c.env);
            }
        }
        for r in u.regions.iter().flatten() {
            f(r.callable.env);
        }
        for b in u.binds.iter().flatten() {
            if let Some(Val::Ref(p)) = b.synced {
                f(p);
            }
        }
        for l in u.lists.iter().flatten() {
            f(l.items.env);
            f(l.key.env);
            f(l.row.env);
            if let Some(c) = l.empty {
                f(c.env);
            }
            for r in &l.rows {
                if let Val::Ref(p) = r.item {
                    f(p);
                }
            }
        }
    });
}
