//! Signals, computed values, effects and owner scopes (SPEC.md §7.1).
//!
//! - A read inside a tracking scope (an effect or a computed) records a
//!   dependency.
//! - A write marks the dependents dirty. The scheduler runs the dirty
//!   effects once per event, after the handler returns (`flush`).
//! - Every effect, signal, handler and node belongs to an owner scope. An
//!   effect gets a new child scope for each run, and disposes the previous
//!   one first. Disposing a scope disposes all that it owns.
//!
//! No `RefCell` borrow is held while user code runs, because user code calls
//! back into the runtime.

use crate::{Callable, Val, invoke};
use crate::global::Global;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

pub type RId = u32;
pub type ScopeId = u32;

/// What an effect does when it runs.
#[derive(Clone, Copy)]
pub enum EffectKind {
    User(Callable),
    Region(u32),
    List(u32),
    Bind(u32),
}

/// Something that a scope must release when it is disposed.
#[derive(Clone, Copy)]
pub enum Cleanup {
    Handler(u32),
    Node(u32),
    List(u32),
    Region(u32),
    Bind(u32),
}

enum RNode {
    Free,
    Signal { value: Val, observers: Vec<RId> },
    Computed { callable: Callable, value: Val, dirty: bool, sources: Vec<RId>, observers: Vec<RId> },
    Effect { kind: EffectKind, dirty: bool, sources: Vec<RId>, child: Option<ScopeId>, scope: ScopeId },
}

struct Scope {
    parent: Option<ScopeId>,
    children: Vec<ScopeId>,
    rnodes: Vec<RId>,
    cleanups: Vec<Cleanup>,
}

struct State {
    nodes: Vec<RNode>,
    free: Vec<RId>,
    scopes: Vec<Option<Scope>>,
    free_scopes: Vec<ScopeId>,
    current_scope: ScopeId,
    observer: Option<RId>,
    queue: VecDeque<RId>,
}

static STATE: Global<State> = Global::new(State {
    nodes: Vec::new(),
    free: Vec::new(),
    scopes: Vec::new(),
    free_scopes: Vec::new(),
    current_scope: 0,
    observer: None,
    queue: VecDeque::new(),
});

const MAX_FLUSH_RUNS: usize = 100_000;

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| {
        if s.scopes.is_empty() {
            // Scope 0 is the root scope. It is never disposed.
            s.scopes.push(Some(Scope { parent: None, children: Vec::new(), rnodes: Vec::new(), cleanups: Vec::new() }));
        }
        f(s)
    })
}

impl State {
    fn alloc(&mut self, node: RNode) -> RId {
        let id = match self.free.pop() {
            Some(id) => {
                self.nodes[id as usize] = node;
                id
            }
            None => {
                self.nodes.push(node);
                (self.nodes.len() - 1) as RId
            }
        };
        let scope = self.current_scope;
        if let Some(s) = self.scopes[scope as usize].as_mut() {
            s.rnodes.push(id);
        }
        id
    }

    fn node(&mut self, id: RId) -> &mut RNode {
        match self.nodes.get_mut(id as usize) {
            Some(n) if !matches!(n, RNode::Free) => n,
            _ => crate::trap("use of a disposed signal"),
        }
    }

    fn track(&mut self, source: RId) {
        let Some(obs) = self.observer else { return };
        match &mut self.nodes[obs as usize] {
            RNode::Computed { sources, .. } | RNode::Effect { sources, .. } => {
                if sources.contains(&source) {
                    return;
                }
                sources.push(source);
            }
            _ => return,
        }
        match &mut self.nodes[source as usize] {
            RNode::Signal { observers, .. } | RNode::Computed { observers, .. } => observers.push(obs),
            _ => {}
        }
    }

    fn unsubscribe(&mut self, id: RId) {
        let sources = match &mut self.nodes[id as usize] {
            RNode::Computed { sources, .. } | RNode::Effect { sources, .. } => core::mem::take(sources),
            _ => return,
        };
        for s in sources {
            if let Some(RNode::Signal { observers, .. } | RNode::Computed { observers, .. }) = self.nodes.get_mut(s as usize) {
                observers.retain(|o| *o != id);
            }
        }
    }

    fn notify(&mut self, id: RId) {
        let observers = match &self.nodes[id as usize] {
            RNode::Signal { observers, .. } | RNode::Computed { observers, .. } => observers.clone(),
            _ => return,
        };
        for o in observers {
            match &mut self.nodes[o as usize] {
                RNode::Effect { dirty, .. } => {
                    if !*dirty {
                        *dirty = true;
                        self.queue.push_back(o);
                    }
                }
                RNode::Computed { dirty, .. } => {
                    if !*dirty {
                        *dirty = true;
                        self.notify(o);
                    }
                }
                _ => {}
            }
        }
    }

    fn new_scope(&mut self, parent: ScopeId) -> ScopeId {
        let scope = Scope { parent: Some(parent), children: Vec::new(), rnodes: Vec::new(), cleanups: Vec::new() };
        let id = match self.free_scopes.pop() {
            Some(id) => {
                self.scopes[id as usize] = Some(scope);
                id
            }
            None => {
                self.scopes.push(Some(scope));
                (self.scopes.len() - 1) as ScopeId
            }
        };
        if let Some(p) = self.scopes[parent as usize].as_mut() {
            p.children.push(id);
        }
        id
    }
}

// ---------------------------------------------------------------------------
// Scopes

pub fn current_scope() -> ScopeId {
    with(|s| s.current_scope)
}

/// Makes a child scope of `parent`.
pub fn new_scope(parent: ScopeId) -> ScopeId {
    with(|s| s.new_scope(parent))
}

/// Runs `f` with `scope` as the owner and with no observer (untracked).
pub fn run_in_scope<R>(scope: ScopeId, f: impl FnOnce() -> R) -> R {
    let (prev_scope, prev_obs) = with(|s| {
        let prev = (s.current_scope, s.observer);
        s.current_scope = scope;
        s.observer = None;
        prev
    });
    let r = f();
    with(|s| {
        s.current_scope = prev_scope;
        s.observer = prev_obs;
    });
    r
}

/// Runs `f` with `scope` as the owner. The observer stays, so reads inside
/// `f` are tracked by the running effect.
pub fn with_scope<R>(scope: ScopeId, f: impl FnOnce() -> R) -> R {
    let prev = with(|s| core::mem::replace(&mut s.current_scope, scope));
    let r = f();
    with(|s| s.current_scope = prev);
    r
}

/// Runs `f` with no observer. Reads inside it are not tracked.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    let prev = with(|s| s.observer.take());
    let r = f();
    with(|s| s.observer = prev);
    r
}

pub fn add_cleanup(c: Cleanup) {
    with(|s| {
        let scope = s.current_scope;
        if let Some(sc) = s.scopes[scope as usize].as_mut() {
            sc.cleanups.push(c);
        }
    });
}

/// Disposes a scope and everything that it owns, children first.
pub fn dispose(scope: ScopeId) {
    if scope == 0 {
        return;
    }
    let Some(sc) = with(|s| s.scopes.get_mut(scope as usize).and_then(Option::take)) else { return };
    for child in sc.children {
        dispose(child);
    }
    for c in sc.cleanups.into_iter().rev() {
        crate::ui::cleanup(c);
    }
    with(|s| {
        // An effect's child scope is a child of this scope, so the loop
        // above disposed it already.
        for id in sc.rnodes {
            #[cfg(feature = "dev")]
            hotreload::unregister(id);
            s.unsubscribe(id);
            s.nodes[id as usize] = RNode::Free;
            s.free.push(id);
        }
        if let Some(parent) = sc.parent.and_then(|p| s.scopes[p as usize].as_mut()) {
            parent.children.retain(|c| *c != scope);
        }
        s.free_scopes.push(scope);
    });
}

// ---------------------------------------------------------------------------
// Signals and computed values

pub fn signal_new(value: Val) -> RId {
    with(|s| s.alloc(RNode::Signal { value, observers: Vec::new() }))
}

pub fn signal_get(id: RId) -> Val {
    with(|s| {
        let v = match s.node(id) {
            RNode::Signal { value, .. } => *value,
            _ => crate::trap("not a signal"),
        };
        s.track(id);
        v
    })
}

pub fn signal_peek(id: RId) -> Val {
    with(|s| match s.node(id) {
        RNode::Signal { value, .. } => *value,
        _ => crate::trap("not a signal"),
    })
}

pub fn signal_set(id: RId, v: Val) {
    with(|s| {
        match s.node(id) {
            RNode::Signal { value, .. } => {
                if value.same(&v) {
                    return;
                }
                *value = v;
            }
            _ => crate::trap("not a signal"),
        }
        s.notify(id);
    });
}

pub fn computed_new(callable: Callable) -> RId {
    with(|s| {
        s.alloc(RNode::Computed { callable, value: Val::None, dirty: true, sources: Vec::new(), observers: Vec::new() })
    })
}

pub fn computed_get(id: RId) -> Val {
    let recompute = with(|s| match s.node(id) {
        RNode::Computed { dirty, callable, .. } => dirty.then_some(*callable),
        _ => crate::trap("not a computed value"),
    });
    if let Some(callable) = recompute {
        let prev = with(|s| {
            s.unsubscribe(id);
            core::mem::replace(&mut s.observer, Some(id))
        });
        let v = invoke(callable, Val::None);
        with(|s| {
            s.observer = prev;
            if let RNode::Computed { value, dirty, .. } = s.node(id) {
                *value = v;
                *dirty = false;
            }
        });
    }
    with(|s| {
        let v = match s.node(id) {
            RNode::Computed { value, .. } => *value,
            _ => unreachable!(),
        };
        s.track(id);
        v
    })
}

// ---------------------------------------------------------------------------
// Effects

/// Makes an effect in the current scope and runs it at once.
pub fn effect_new(kind: EffectKind) -> RId {
    let id = with(|s| {
        let scope = s.current_scope;
        s.alloc(RNode::Effect { kind, dirty: true, sources: Vec::new(), child: None, scope })
    });
    run_effect(id);
    id
}

fn run_effect(id: RId) {
    let Some((kind, old_child, scope)) = with(|s| match s.nodes.get_mut(id as usize) {
        Some(RNode::Effect { kind, dirty, child, scope, .. }) => {
            *dirty = false;
            Some((*kind, child.take(), *scope))
        }
        _ => None,
    }) else {
        return;
    };
    with(|s| s.unsubscribe(id));
    if let Some(c) = old_child {
        dispose(c);
    }
    let (child, prev_scope, prev_obs) = with(|s| {
        let child = s.new_scope(scope);
        if let RNode::Effect { child: c, .. } = &mut s.nodes[id as usize] {
            *c = Some(child);
        }
        let prev = (s.current_scope, s.observer);
        s.current_scope = child;
        s.observer = Some(id);
        (child, prev.0, prev.1)
    });
    let _ = child;
    crate::ui::run_effect(kind);
    with(|s| {
        s.current_scope = prev_scope;
        s.observer = prev_obs;
    });
}

// ---------------------------------------------------------------------------
// Hot reload (SPEC.md §13, dev builds only)
//
// Each module-level `signal(...)` gets a stable key (a hash of its module
// index and declaration name, computed at compile time) and a shape code
// (a small integer that identifies the value's wire shape: f64, int,
// bool or string). `lower.rs` emits a `sig_register` call for it right
// after `sig_new`, but only when compiling in dev mode, so this registry
// and the code below never run in a release build. Component-local
// signals (inside a component function, a loop, or a closure) are never
// registered, so they always start fresh after a reload.
// Component-local signals (SPEC.md §13): a signal declared inside a
// component function or a `List` row closure is keyed by the declaring
// function's name plus the n-th registrable `signal(...)` in it (computed
// at compile time, see `lower.rs`), combined at *register* time with the
// current owner scope's "instance path". The instance path is 0 for the
// root scope and for any scope that does not explicitly set one; a `List`
// row scope sets its path from the row's key, combined with its owner's
// path, so sibling rows keyed differently get different final keys and a
// signal declared inside nested components keeps the path of the row (or
// root) they ultimately run under (components do not create their own
// scope, so nested component calls inherit the current scope's path).
#[cfg(feature = "dev")]
mod hotreload {
    use super::{RId, ScopeId, Val, signal_peek, signal_set};
    use alloc::vec::Vec;

    struct Entry {
        key: u32,
        shape: u32,
        id: RId,
    }

    static REGISTRY: crate::global::Global<Vec<Entry>> = crate::global::Global::new(Vec::new());
    static PATHS: crate::global::Global<Vec<u32>> = crate::global::Global::new(Vec::new());

    fn ensure(scope: ScopeId, paths: &mut Vec<u32>) {
        let i = scope as usize;
        if paths.len() <= i {
            paths.resize(i + 1, 0);
        }
    }

    /// The instance path of `scope` (0 if never set).
    pub fn path_of(scope: ScopeId) -> u32 {
        PATHS.with(|p| {
            ensure(scope, p);
            p[scope as usize]
        })
    }

    /// Sets the instance path of `scope`. `List` rows call this right
    /// after creating their scope, combining their owner's path with a
    /// hash of the row key.
    pub fn set_path(scope: ScopeId, path: u32) {
        PATHS.with(|p| {
            ensure(scope, p);
            p[scope as usize] = path;
        });
    }

    /// A small, deterministic mix of a declaration key (or path) with an
    /// instance discriminator (a row key hash, or a child path). Not
    /// cryptographic; it only needs to separate sibling instances.
    pub fn combine(a: u32, b: u32) -> u32 {
        (a ^ b).wrapping_mul(0x0100_0193)
    }

    pub fn register(id: RId, key: i32, shape: i32) {
        let key = combine(key as u32, path_of(super::current_scope()));
        REGISTRY.with(|r| r.push(Entry { key, shape: shape as u32, id }));
    }

    /// Drops the registry entry for a disposed signal, so a removed
    /// `List` row's state does not leak and a reused runtime id does not
    /// wrongly match a future, unrelated signal.
    pub fn unregister(id: RId) {
        REGISTRY.with(|r| r.retain(|e| e.id != id));
    }

    /// Writes `(count, then one (key, shape, tag, payload) per entry)`.
    /// Only f64, i32 (bool/int) and string values are written; other
    /// shapes are not registered in the first place (see `shape_code` in
    /// `lower.rs`).
    pub fn snapshot() -> Vec<u8> {
        REGISTRY.with(|r| {
            let mut out = Vec::new();
            out.extend_from_slice(&(r.len() as u32).to_le_bytes());
            for e in r.iter() {
                out.extend_from_slice(&e.key.to_le_bytes());
                out.extend_from_slice(&e.shape.to_le_bytes());
                write_val(&mut out, signal_peek(e.id));
            }
            out
        })
    }

    fn write_val(out: &mut Vec<u8>, v: Val) {
        match v {
            Val::None => out.push(0),
            Val::F64(f) => {
                out.push(1);
                out.extend_from_slice(&f.to_le_bytes());
            }
            Val::I32(i) => {
                out.push(2);
                out.extend_from_slice(&i.to_le_bytes());
            }
            Val::Ref(p) => {
                // Only strings are registered (shape_code rejects other
                // ref-repr types), so this is always a string object.
                out.push(3);
                let s = crate::strings::as_str(p);
                out.extend_from_slice(&(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
        }
    }

    /// Restores every registered signal whose key and shape match an
    /// entry in `buf`. A mismatch (missing key, or a shape that changed)
    /// is skipped silently; dev builds log it (SPEC.md §13).
    pub fn restore(buf: &[u8]) {
        let Some(mut p) = Reader::new(buf) else { return };
        let Some(count) = p.u32() else { return };
        for _ in 0..count {
            let (Some(key), Some(shape)) = (p.u32(), p.u32()) else { return };
            let Some(tag) = p.u8() else { return };
            let val = match tag {
                0 => Some(Val::None),
                1 => p.f64().map(Val::F64),
                2 => p.i32().map(Val::I32),
                3 => p.str_val(),
                _ => None,
            };
            let Some(val) = val else { return };
            let found = REGISTRY.with(|r| r.iter().find(|e| e.key == key && e.shape == shape).map(|e| e.id));
            match found {
                Some(id) => signal_set(id, val),
                None => crate::log("hot reload: a signal's key or shape changed; it did not restore"),
            }
        }
    }

    /// A tiny cursor, separate from `plinth_protocol::Reader` (which does
    /// not expose raw string bytes), since this format is private to the
    /// runtime and the dev CLI.
    struct Reader<'a> {
        buf: &'a [u8],
        pos: usize,
    }

    impl<'a> Reader<'a> {
        fn new(buf: &'a [u8]) -> Option<Self> {
            Some(Self { buf, pos: 0 })
        }
        fn u8(&mut self) -> Option<u8> {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            Some(b)
        }
        fn u32(&mut self) -> Option<u32> {
            let s = self.buf.get(self.pos..self.pos + 4)?;
            self.pos += 4;
            Some(u32::from_le_bytes(s.try_into().unwrap()))
        }
        fn f64(&mut self) -> Option<f64> {
            let s = self.buf.get(self.pos..self.pos + 8)?;
            self.pos += 8;
            Some(f64::from_le_bytes(s.try_into().unwrap()))
        }
        fn i32(&mut self) -> Option<i32> {
            self.u32().map(|v| v as i32)
        }
        fn str_val(&mut self) -> Option<Val> {
            let len = self.u32()? as usize;
            let bytes = self.buf.get(self.pos..self.pos + len)?;
            self.pos += len;
            let s = core::str::from_utf8(bytes).ok()?;
            Some(Val::Ref(crate::strings::from_str(s)))
        }
    }
}

#[cfg(feature = "dev")]
pub use hotreload::{register as sig_register, restore as sig_restore, snapshot as sig_snapshot};
#[cfg(feature = "dev")]
pub use hotreload::{combine as hotreload_combine, path_of as hotreload_path_of, set_path as hotreload_set_path};

/// Runs the dirty effects until no effect is dirty.
pub fn flush() {
    let mut runs = 0;
    while let Some(id) = with(|s| s.queue.pop_front()) {
        let dirty = with(|s| matches!(s.nodes.get(id as usize), Some(RNode::Effect { dirty: true, .. })));
        if dirty {
            runs += 1;
            if runs > MAX_FLUSH_RUNS {
                crate::trap("too many effect runs; an effect probably writes a signal that it reads");
            }
            run_effect(id);
        }
    }
}

/// Gives every heap reference that the reactive graph holds.
pub fn trace_roots(f: &mut dyn FnMut(u32)) {
    with(|s| {
        for n in &s.nodes {
            match n {
                RNode::Signal { value: Val::Ref(p), .. } => f(*p),
                RNode::Computed { callable, value, .. } => {
                    f(callable.env);
                    if let Val::Ref(p) = value {
                        f(*p);
                    }
                }
                RNode::Effect { kind: EffectKind::User(c), .. } => f(c.env),
                _ => {}
            }
        }
    });
}
