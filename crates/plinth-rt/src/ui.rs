//! The guest side of the semantic tree: node ids, the op buffer, dynamic
//! child regions, event handlers, two-way bindings and the keyed list
//! reconciler (SPEC.md §7.2, §7.3, §8.4).

use crate::global::Global;
use crate::reactive::{self, Cleanup, EffectKind, ScopeId};
use crate::{Callable, Val, invoke, strings};
use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;
use plinth_protocol::{ControlKind, NodeId, Op, Value, Writer, event, nav_kind, prop};

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
    /// A user callback, and the node and event it listens to.
    User(Callable, NodeId, u16),
    Bind(u32),
}

#[derive(Clone, Copy, PartialEq)]
pub enum BindKind {
    Str,
    Bool,
    Num,
}

struct Bind {
    node: NodeId,
    signal: u32,
    kind: BindKind,
    /// The last value that the host and the guest agree on.
    synced: Option<Val>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    /// The current child order in the host.
    order: Vec<NodeId>,
    empty_node: Option<(NodeId, ScopeId)>,
}

struct Ui {
    ops: Writer,
    next_node: NodeId,
    free_nodes: Vec<NodeId>,
    /// Indexed by node id.
    slots: Vec<Vec<Slot>>,
    regions: Vec<Option<Region>>,
    /// Indexed by handler id - 1.
    handlers: Vec<Option<Handler>>,
    free_handlers: Vec<u32>,
    binds: Vec<Option<Bind>>,
    lists: Vec<Option<List>>,
    /// While a `change` event runs: its node and the value that the host
    /// shows already. `set_prop` does not send this value back (no echo of
    /// a one-way `value` plus `onChange`, SPEC.md §8.4).
    host_value: Option<(NodeId, Value)>,
    /// The last value of each prop and the text that the host got, by
    /// node id (docs/GAPS.md G8). An effect that gives the same value again
    /// sends no op. `value` is not in it: the user can change that one in
    /// the host, so the app must be able to set it again.
    sent: Vec<Vec<(u16, Value)>>,
}

/// The key of the text in `Ui::sent` (no prop has this id).
const TEXT_KEY: u16 = u16::MAX;

static UI: Global<Ui> = Global::new(Ui {
    ops: Writer::new(),
    next_node: 1,
    free_nodes: Vec::new(),
    slots: Vec::new(),
    regions: Vec::new(),
    handlers: Vec::new(),
    free_handlers: Vec::new(),
    binds: Vec::new(),
    lists: Vec::new(),
    host_value: None,
    sent: Vec::new(),
});

fn with<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(f)
}

impl Ui {
    /// Records `value` as sent for (`id`, `key`); false if the host has it already.
    fn changed(&mut self, id: NodeId, key: u16, value: &Value) -> bool {
        let i = id as usize;
        if self.sent.len() <= i {
            self.sent.resize_with(i + 1, Vec::new);
        }
        let props = &mut self.sent[i];
        match props.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) if v == value => false,
            Some((_, v)) => {
                *v = value.clone();
                true
            }
            None => {
                props.push((key, value.clone()));
                true
            }
        }
    }

    fn slots_mut(&mut self, parent: NodeId) -> &mut Vec<Slot> {
        let i = parent as usize;
        if self.slots.len() <= i {
            self.slots.resize_with(i + 1, Vec::new);
        }
        &mut self.slots[i]
    }

    fn slots(&self, parent: NodeId) -> &[Slot] {
        self.slots.get(parent as usize).map(Vec::as_slice).unwrap_or(&[])
    }

    fn add_handler(&mut self, h: Handler) -> u32 {
        match self.free_handlers.pop() {
            Some(id) => {
                self.handlers[id as usize - 1] = Some(h);
                id
            }
            None => {
                self.handlers.push(Some(h));
                self.handlers.len() as u32
            }
        }
    }

    fn handler(&self, id: u32) -> Option<&Handler> {
        self.handlers.get((id as usize).wrapping_sub(1)).and_then(Option::as_ref)
    }
}

/// Returns the pending op buffer and leaves it empty.
pub fn take_ops() -> Vec<u8> {
    with(|u| u.ops.take())
}

/// Dev builds only (SPEC.md §13): queues the hot-reload snapshot as an
/// `Op::Snapshot`, so the host gets it back in the next commit.
#[cfg(feature = "dev")]
pub fn push_snapshot_op(bytes: alloc::vec::Vec<u8>) {
    with(|u| u.ops.op(&plinth_protocol::Op::Snapshot { bytes }));
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
        if let Some(sent) = u.sent.get_mut(id as usize) {
            sent.clear(); // a new node: the host has none of its props
        }
        // A new node with the id of the changed one (ids are used again)
        // does not show the value of the change.
        if matches!(&u.host_value, Some((n, _)) if *n == id) {
            u.host_value = None;
        }
        id
    });
    reactive::add_cleanup(Cleanup::Node(id));
    id
}

pub fn set_prop(id: NodeId, prop: u16, value: Value) {
    with(|u| {
        if prop == prop::VALUE && matches!(&u.host_value, Some((n, v)) if *n == id && *v == value) {
            return; // The host shows this value already.
        }
        if prop != prop::VALUE && !u.changed(id, prop, &value) {
            return; // The host has this value already (G8).
        }
        u.ops.op(&Op::SetProp { id, prop, value })
    });
}

/// Ends one UI event: from now on, `set_prop` sends every value again.
pub fn end_event() {
    with(|u| u.host_value = None);
}

pub fn set_text(id: NodeId, text: &str) {
    with(|u| {
        let value = Value::Str(text.to_owned());
        if u.changed(id, TEXT_KEY, &value) {
            u.ops.op(&Op::Text { id, value });
        }
    });
}

/// Appends a static child.
pub fn append(parent: NodeId, child: NodeId) {
    with(|u| {
        u.slots_mut(parent).push(Slot::Node(child));
        u.ops.op(&Op::Insert { parent, id: child, before: 0 });
    });
}

pub fn set_root(screen: u32, id: NodeId) {
    with(|u| u.ops.op(&Op::SetRoot { screen, id }));
}

pub fn navigate(screen: u32) {
    with(|u| u.ops.op(&Op::Navigate { kind: nav_kind::SELECT_PRIMARY, screen, args: Value::Null }));
}

/// Declares `screen` as a primary (top-level) destination. `lower.rs` emits
/// this once per primary screen, before any other navigate op (UI API 1.2).
pub fn mark_primary(screen: u32) {
    with(|u| u.ops.op(&Op::Navigate { kind: nav_kind::MARK_PRIMARY, screen, args: Value::Null }));
}

/// `navigate.push(screen)` (UI API 1.2): pushes `screen` onto the stack of
/// the current primary tab.
pub fn navigate_push(screen: u32) {
    with(|u| u.ops.op(&Op::Navigate { kind: nav_kind::PUSH, screen, args: Value::Null }));
}

/// `navigate.back()` (UI API 1.2): pops the current primary tab's stack.
pub fn navigate_back() {
    with(|u| u.ops.op(&Op::Navigate { kind: nav_kind::BACK, screen: 0, args: Value::Null }));
}

/// The first node after slot `index` of `parent`, or 0 (append).
fn node_after(u: &Ui, parent: NodeId, index: usize) -> NodeId {
    for slot in &u.slots(parent)[index + 1..] {
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
        u.slots_mut(parent).push(Slot::Region(id));
        id
    });
    reactive::add_cleanup(Cleanup::Region(id));
    reactive::effect_new(EffectKind::Region(id));
}

fn run_region(id: u32) {
    let Some((callable, old_node, old_content)) =
        with(|u| u.regions[id as usize].as_mut().map(|r| (r.callable, r.node, r.content.take())))
    else {
        return;
    };
    // Remove the old content from the host and dispose its scope BEFORE
    // building new content. Disposing frees any node ids the old content
    // held (Cleanup::Node pushes them to `free_nodes`), so if we built the
    // new content first it could be assigned an id the host still thinks
    // is in use. Removing the old node first also means the host sees
    // Remove before any Create that reuses its id.
    if old_node != 0 {
        with(|u| u.ops.op(&Op::Remove { id: old_node }));
    }
    if let Some(c) = old_content {
        reactive::dispose(c);
    }
    // The content gets its own scope, which the next run disposes. The
    // region effect tracks the reads of the callable (the condition).
    let scope = reactive::new_scope(reactive::current_scope());
    let node = reactive::with_scope(scope, || match invoke(callable, Val::None) {
        Val::I32(n) => n as NodeId,
        _ => 0,
    });
    with(|u| {
        let parent = u.regions[id as usize].as_ref().map(|r| r.parent).unwrap_or(0);
        if node != 0 {
            let index = u.slots(parent).iter().position(|s| *s == Slot::Region(id)).unwrap_or(0);
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
        let h = u.add_handler(Handler::User(callable, node, ev));
        u.ops.op(&Op::Listen { id: node, event: ev, handler: h });
        h
    });
    reactive::add_cleanup(Cleanup::Handler(h));
}

/// A two-way binding of `value` to a signal (SPEC.md §8.4).
pub fn bind(node: NodeId, signal: u32, kind: BindKind) {
    let (id, h) = with(|u| {
        let id = u.binds.len() as u32;
        u.binds.push(Some(Bind { node, signal, kind, synced: None }));
        let h = u.add_handler(Handler::Bind(id));
        u.ops.op(&Op::Listen { id: node, event: event::CHANGE, handler: h });
        (id, h)
    });
    reactive::add_cleanup(Cleanup::Bind(id));
    reactive::add_cleanup(Cleanup::Handler(h));
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
    let action = with(|u| match u.handler(handler) {
        Some(&Handler::User(c, node, ev)) => {
            if ev == event::CHANGE {
                u.host_value = Some((node, value.clone()));
            }
            Some(Action::User(c))
        }
        Some(Handler::Bind(b)) => u.binds[*b as usize].as_ref().map(|bind| Action::Bind(*b, bind.kind)),
        None => None,
    });
    match action {
        // A handler for a node that was removed in the same batch.
        None => {}
        // A list value (UI API 1.12 pointer events: `[x, y]`) gives the
        // callback one argument per item.
        Some(Action::User(c)) => match value {
            Value::List(items) => {
                let mut args = [Val::None; crate::MAX_ARGS];
                for (slot, v) in args.iter_mut().zip(items) {
                    *slot = value_to_val(v);
                }
                reactive::untracked(|| crate::invoke_args(c, &args[..items.len().min(crate::MAX_ARGS)]));
            }
            _ => {
                reactive::untracked(|| invoke(c, value_to_val(value)));
            }
        },
        Some(Action::Bind(b, kind)) => {
            let v = match (kind, value) {
                (BindKind::Bool, Value::Bool(x)) => Val::I32(*x as i32),
                (BindKind::Str, Value::Str(_)) => value_to_val(value),
                (BindKind::Num, Value::Number(_) | Value::Int(_)) => value_to_val(value),
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
        u.lists.push(Some(List {
            node,
            owner,
            items,
            key,
            row,
            empty,
            rows: Vec::new(),
            order: Vec::new(),
            empty_node: None,
        }));
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

/// A hash of a `List` row key, for hot reload's instance paths (SPEC.md
/// §13). Dev builds only; FNV-1a, the same algorithm the compiler uses
/// for declaration keys.
#[cfg(feature = "dev")]
fn key_hash(k: &Key) -> u32 {
    fn fnv1a_bytes(bytes: &[u8]) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for b in bytes {
            h ^= *b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
        h
    }
    match k {
        Key::Num(bits) => fnv1a_bytes(&bits.to_le_bytes()),
        Key::Str(s) => fnv1a_bytes(s.as_bytes()),
    }
}

/// Marks the elements of `seq` (old positions of the rows in their new
/// order, `u32::MAX` for a row with no old position) that belong to the
/// longest strictly increasing subsequence of the non-`MAX` elements. Those
/// rows are already in relative order and need no `Op::Move`; this is the
/// standard keyed-diff minimal-moves trick. O(n log n) (patience sorting
/// with parent pointers for reconstruction).
fn longest_increasing_subsequence(seq: &[u32]) -> alloc::vec::Vec<bool> {
    // Indices into `seq` of the candidates (new rows cannot anchor the LIS).
    let idxs: Vec<usize> = (0..seq.len()).filter(|&i| seq[i] != u32::MAX).collect();
    // `tails[k]` is the index (into `idxs`) of the smallest tail value of an
    // increasing subsequence of length k + 1 found so far.
    let mut tails: Vec<usize> = Vec::new();
    // `prev[k]` is the predecessor of `idxs[k]` in its subsequence, as an
    // index into `idxs`, or -1 at the start of a subsequence.
    let mut prev: Vec<i32> = alloc::vec::Vec::new();
    prev.resize(idxs.len(), -1);
    for (k, &i) in idxs.iter().enumerate() {
        let v = seq[i];
        let pos = tails.partition_point(|&t| seq[idxs[t]] < v);
        if pos > 0 {
            prev[k] = tails[pos - 1] as i32;
        }
        if pos == tails.len() {
            tails.push(k);
        } else {
            tails[pos] = k;
        }
    }
    let mut in_lis = alloc::vec::Vec::new();
    in_lis.resize(seq.len(), false);
    let mut k = tails.last().copied();
    while let Some(kk) = k {
        in_lis[idxs[kk]] = true;
        k = if prev[kk] >= 0 { Some(prev[kk] as usize) } else { None };
    }
    in_lis
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

    let (old_rows, prev_order) = with(|u| {
        let l = u.lists[id as usize].as_mut().unwrap();
        (core::mem::take(&mut l.rows), core::mem::take(&mut l.order))
    });
    let mut old: Vec<Option<Row>> = old_rows.into_iter().map(Some).collect();
    let mut rows = Vec::with_capacity(items.len());
    // Whether `rows[i]` is the same row (same node) `prev_order` already
    // has in it. A brand-new row's node id can coincidentally equal a
    // removed row's freed id (ids are reused), so this must be tracked
    // explicitly rather than inferred later by matching `r.node` against
    // `prev_order`.
    let mut reused = Vec::with_capacity(items.len());
    // A sorted index of (key, position in `old`) for O(log n) reuse lookup
    // instead of a linear scan: scanning `old` per new item made init,
    // filter and append O(n^2) for large lists (SPEC.md §7.3, Q6).
    let mut old_index: Vec<(Key, usize)> =
        old.iter().enumerate().map(|(i, r)| (r.as_ref().unwrap().key.clone(), i)).collect();
    old_index.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    reactive::untracked(|| {
        for item in items {
            let key = to_key(invoke(key_c, item));
            let reuse = old_index
                .binary_search_by(|(k, _)| k.cmp(&key))
                .ok()
                .map(|found| old_index[found].1)
                .filter(|i| old[*i].is_some());
            match reuse.and_then(|i| old[i].take()) {
                // The same key and the same item: keep the row.
                Some(r) if r.item.same(&item) => {
                    rows.push(r);
                    reused.push(true);
                }
                stale => {
                    if let Some(r) = stale {
                        remove_row(r);
                    }
                    let scope = reactive::new_scope(owner);
                    #[cfg(feature = "dev")]
                    {
                        let base = reactive::hotreload_path_of(owner);
                        reactive::hotreload_set_path(scope, reactive::hotreload_combine(base, key_hash(&key)));
                    }
                    let row_node = reactive::run_in_scope(scope, || match invoke(row_c, item) {
                        Val::I32(n) => n as NodeId,
                        _ => crate::trap("a List row must return an element"),
                    });
                    rows.push(Row { key, item, node: row_node, scope });
                    reused.push(false);
                }
            }
        }
    });
    for r in old.into_iter().flatten() {
        remove_row(r);
    }

    // Put the row nodes in order with insert and move ops: the standard
    // keyed-diff method. `old_pos[i]` is the index in `prev_order` of
    // `rows[i]`'s node, or `u32::MAX` for a brand-new row. The rows whose
    // old positions form the longest increasing subsequence are already in
    // relative order, so they need no `Move`; every other row gets one
    // `Move` (or `Insert` if new). Walking from the end means each op's
    // `before` is a node already placed at its final position. O(n log n),
    // versus the old O(n) `position`/`remove`/`insert` per out-of-place row.
    let mut old_pos_sorted: Vec<(NodeId, u32)> =
        prev_order.iter().enumerate().map(|(i, n)| (*n, i as u32)).collect();
    old_pos_sorted.sort_unstable_by_key(|(n, _)| *n);
    let old_pos: Vec<u32> = rows
        .iter()
        .zip(reused.iter())
        .map(|(r, &was_reused)| {
            if !was_reused {
                return u32::MAX;
            }
            old_pos_sorted.binary_search_by_key(&r.node, |(n, _)| *n).map(|found| old_pos_sorted[found].1).unwrap_or(u32::MAX)
        })
        .collect();
    let in_lis = longest_increasing_subsequence(&old_pos);
    let current: Vec<NodeId> = rows.iter().map(|r| r.node).collect();
    with(|u| {
        let mut before: NodeId = 0;
        for i in (0..rows.len()).rev() {
            let r = &rows[i];
            if in_lis[i] {
                // Already in the right relative order; no op needed, but
                // it is the next `before` for an earlier out-of-place row.
            } else if old_pos[i] == u32::MAX {
                u.ops.op(&Op::Insert { parent: node, id: r.node, before });
            } else {
                u.ops.op(&Op::Move { parent: node, id: r.node, before });
            }
            before = r.node;
        }
    });

    // The empty placeholder.
    let has_rows = !rows.is_empty();
    let empty_node = with(|u| {
        let l = u.lists[id as usize].as_mut().unwrap();
        l.rows = rows;
        l.order = current;
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
        Cleanup::User(c) => {
            reactive::untracked(|| invoke(c, Val::None));
        }
        Cleanup::Handler(h) => with(|u| {
            if let Some(slot) = u.handlers.get_mut((h as usize).wrapping_sub(1))
                && slot.take().is_some()
            {
                u.free_handlers.push(h);
            }
        }),
        Cleanup::Node(n) => with(|u| {
            if let Some(s) = u.slots.get_mut(n as usize) {
                s.clear();
            }
            if let Some(sent) = u.sent.get_mut(n as usize) {
                sent.clear();
            }
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
        for h in u.handlers.iter().flatten() {
            if let Handler::User(c, _, _) = h {
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
