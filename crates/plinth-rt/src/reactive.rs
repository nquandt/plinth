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
use std::cell::RefCell;
use std::collections::VecDeque;

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

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        nodes: Vec::new(),
        free: Vec::new(),
        // Scope 0 is the root scope. It is never disposed.
        scopes: vec![Some(Scope { parent: None, children: Vec::new(), rnodes: Vec::new(), cleanups: Vec::new() })],
        free_scopes: Vec::new(),
        current_scope: 0,
        observer: None,
        queue: VecDeque::new(),
    });
}

const MAX_FLUSH_RUNS: usize = 100_000;

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with_borrow_mut(f)
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
            RNode::Computed { sources, .. } | RNode::Effect { sources, .. } => std::mem::take(sources),
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
    let prev = with(|s| std::mem::replace(&mut s.current_scope, scope));
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
            std::mem::replace(&mut s.observer, Some(id))
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
