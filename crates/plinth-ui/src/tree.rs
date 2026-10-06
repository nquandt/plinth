//! The semantic tree arena (SPEC.md §9.3) and the op applier (SPEC.md §8.3).
//!
//! This module has no gpui dependency, so tests can drive it directly.

use plinth_protocol::{ControlKind, NodeId, Op, Value, decode_ops, nav_kind};
use std::collections::BTreeMap;

/// One node of the semantic tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    /// `None` when the host does not know the kind (a UI API version mismatch).
    pub kind: Option<ControlKind>,
    pub raw_kind: u16,
    pub parent: NodeId,
    pub children: Vec<NodeId>,
    pub props: Vec<(u16, Value)>,
    pub text: Option<String>,
    /// `(event, handler)` pairs.
    pub listeners: Vec<(u16, u32)>,
}

impl Node {
    pub fn prop(&self, prop: u16) -> Option<&Value> {
        self.props.iter().find(|(p, _)| *p == prop).map(|(_, v)| v)
    }

    pub fn str_prop(&self, prop: u16) -> Option<&str> {
        self.prop(prop).and_then(Value::as_str)
    }

    pub fn bool_prop(&self, prop: u16) -> bool {
        self.prop(prop).and_then(Value::as_bool).unwrap_or(false)
    }

    pub fn enum_prop(&self, prop: u16) -> u16 {
        self.prop(prop).and_then(Value::as_enum).unwrap_or(0)
    }

    pub fn num_prop(&self, prop: u16) -> Option<f64> {
        self.prop(prop).and_then(Value::as_number)
    }

    pub fn handler(&self, event: u16) -> Option<u32> {
        self.listeners.iter().find(|(e, _)| *e == event).map(|(_, h)| *h)
    }

    fn set_prop(&mut self, prop: u16, value: Value) {
        match self.props.iter_mut().find(|(p, _)| *p == prop) {
            Some(slot) => slot.1 = value,
            None => self.props.push((prop, value)),
        }
    }
}

/// A problem in a commit. The applier skips the bad op and continues.
#[derive(Debug, Clone, PartialEq)]
pub struct OpError {
    pub index: usize,
    pub message: String,
}

#[derive(Default)]
pub struct Tree {
    /// Indexed by guest node id. Slot 0 is never used.
    nodes: Vec<Option<Node>>,
    /// Screen index → root node.
    roots: BTreeMap<u32, NodeId>,
    /// The selected primary screen.
    pub current_screen: u32,
    /// Ids that were removed since the last call to `take_removed`.
    removed: Vec<NodeId>,
}

/// The upper limit of node ids. It stops a hostile guest from making the
/// host allocate a huge arena with one op.
pub const MAX_NODE_ID: NodeId = 1 << 22;

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id as usize).and_then(Option::as_ref)
    }

    fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id as usize).and_then(Option::as_mut)
    }

    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `(screen index, root node)` pairs in screen order.
    pub fn screens(&self) -> impl Iterator<Item = (u32, NodeId)> + '_ {
        self.roots.iter().map(|(s, id)| (*s, *id))
    }

    pub fn current_root(&self) -> Option<&Node> {
        let id = self.roots.get(&self.current_screen).or_else(|| self.roots.values().next())?;
        self.get(*id)
    }

    /// Returns the node ids that were removed since the last call. The
    /// renderer uses them to drop per-node state.
    pub fn take_removed(&mut self) -> Vec<NodeId> {
        std::mem::take(&mut self.removed)
    }

    /// The host-side half of a two-way binding: the host shows the new value
    /// at once, before the guest sees the event (SPEC.md §8.4).
    pub fn set_local_prop(&mut self, id: NodeId, prop: u16, value: Value) {
        if let Some(node) = self.get_mut(id) {
            node.set_prop(prop, value);
        }
    }

    /// Decodes and applies one commit. A malformed buffer changes nothing.
    /// A well-formed op that refers to a bad node is skipped and reported.
    pub fn apply(&mut self, buf: &[u8]) -> Result<Vec<OpError>, plinth_protocol::DecodeError> {
        let ops = decode_ops(buf)?;
        let mut errors = Vec::new();
        for (index, op) in ops.into_iter().enumerate() {
            if let Err(message) = self.apply_op(op) {
                errors.push(OpError { index, message });
            }
        }
        Ok(errors)
    }

    fn apply_op(&mut self, op: Op) -> Result<(), String> {
        match op {
            Op::Create { id, kind } => {
                if id == 0 || id >= MAX_NODE_ID {
                    return Err(format!("create: invalid id {id}"));
                }
                if self.get(id).is_some() {
                    return Err(format!("create: id {id} is in use"));
                }
                if self.nodes.len() <= id as usize {
                    self.nodes.resize(id as usize + 1, None);
                }
                let control = ControlKind::from_u16(kind);
                if control.is_none() {
                    log::error!("unknown control kind {kind} for node {id}; rendering a placeholder");
                }
                self.nodes[id as usize] = Some(Node {
                    id,
                    kind: control,
                    raw_kind: kind,
                    parent: 0,
                    children: Vec::new(),
                    props: Vec::new(),
                    text: None,
                    listeners: Vec::new(),
                });
                Ok(())
            }
            Op::Remove { id } => {
                let parent = self.node(id)?.parent;
                self.detach(id, parent);
                self.roots.retain(|_, root| *root != id);
                self.free_subtree(id);
                Ok(())
            }
            Op::Insert { parent, id, before } | Op::Move { parent, id, before } => {
                self.node(parent)?;
                if id == parent || self.is_ancestor(id, parent) {
                    return Err(format!("insert: node {id} cannot go inside itself"));
                }
                let old_parent = self.node(id)?.parent;
                self.detach(id, old_parent);
                let children = &mut self.get_mut(parent).unwrap().children;
                let at = match before {
                    0 => children.len(),
                    b => children.iter().position(|c| *c == b).unwrap_or(children.len()),
                };
                children.insert(at, id);
                self.get_mut(id).unwrap().parent = parent;
                Ok(())
            }
            Op::SetProp { id, prop, value } => {
                self.node_mut(id)?.set_prop(prop, value);
                Ok(())
            }
            Op::Listen { id, event, handler } => {
                let node = self.node_mut(id)?;
                node.listeners.retain(|(e, _)| *e != event);
                node.listeners.push((event, handler));
                Ok(())
            }
            Op::Unlisten { id, event } => {
                self.node_mut(id)?.listeners.retain(|(e, _)| *e != event);
                Ok(())
            }
            Op::SetRoot { screen, id } => {
                if id == 0 {
                    self.roots.remove(&screen);
                } else {
                    self.node(id)?;
                    self.roots.insert(screen, id);
                }
                Ok(())
            }
            Op::Navigate { kind, screen, .. } => match kind {
                nav_kind::SELECT_PRIMARY | nav_kind::REPLACE => {
                    self.current_screen = screen;
                    Ok(())
                }
                // The stack navigator comes with M2.
                _ => Err(format!("navigate: kind {kind} is not supported yet")),
            },
            Op::Text { id, value } => {
                let node = self.node_mut(id)?;
                node.text = match value {
                    Value::Str(s) => Some(s),
                    Value::Null => None,
                    other => return Err(format!("text: expected a string, got {other:?}")),
                };
                Ok(())
            }
        }
    }

    fn node(&self, id: NodeId) -> Result<&Node, String> {
        self.get(id).ok_or_else(|| format!("no node with id {id}"))
    }

    fn node_mut(&mut self, id: NodeId) -> Result<&mut Node, String> {
        self.get_mut(id).ok_or_else(|| format!("no node with id {id}"))
    }

    fn is_ancestor(&self, ancestor: NodeId, mut id: NodeId) -> bool {
        while let Some(node) = self.get(id) {
            if node.parent == ancestor {
                return true;
            }
            id = node.parent;
        }
        false
    }

    fn detach(&mut self, id: NodeId, parent: NodeId) {
        if let Some(p) = self.get_mut(parent) {
            p.children.retain(|c| *c != id);
        }
        if let Some(n) = self.get_mut(id) {
            n.parent = 0;
        }
    }

    fn free_subtree(&mut self, id: NodeId) {
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.nodes.get_mut(id as usize).and_then(Option::take) {
                stack.extend(node.children);
                self.removed.push(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plinth_protocol::{Writer, event, prop};

    fn commit(tree: &mut Tree, ops: &[Op]) -> Vec<OpError> {
        let mut w = Writer::new();
        for op in ops {
            w.op(op);
        }
        tree.apply(w.as_bytes()).unwrap()
    }

    fn create(id: NodeId, kind: ControlKind) -> Op {
        Op::Create { id, kind: kind as u16 }
    }

    fn insert(parent: NodeId, id: NodeId, before: NodeId) -> Op {
        Op::Insert { parent, id, before }
    }

    #[test]
    fn builds_and_orders_children() {
        let mut t = Tree::new();
        let errors = commit(
            &mut t,
            &[
                create(1, ControlKind::Screen),
                create(2, ControlKind::Text),
                create(3, ControlKind::Text),
                create(4, ControlKind::Text),
                insert(1, 2, 0),
                insert(1, 3, 0),
                insert(1, 4, 3),
                Op::SetRoot { screen: 0, id: 1 },
            ],
        );
        assert!(errors.is_empty());
        assert_eq!(t.get(1).unwrap().children, vec![2, 4, 3]);
        assert_eq!(t.current_root().unwrap().id, 1);

        commit(&mut t, &[Op::Move { parent: 1, id: 3, before: 2 }]);
        assert_eq!(t.get(1).unwrap().children, vec![3, 2, 4]);
    }

    #[test]
    fn remove_frees_the_subtree() {
        let mut t = Tree::new();
        commit(
            &mut t,
            &[
                create(1, ControlKind::Screen),
                create(2, ControlKind::List),
                create(3, ControlKind::Row),
                insert(1, 2, 0),
                insert(2, 3, 0),
                Op::Remove { id: 2 },
            ],
        );
        assert!(t.get(2).is_none() && t.get(3).is_none());
        assert!(t.get(1).unwrap().children.is_empty());
        let mut removed = t.take_removed();
        removed.sort();
        assert_eq!(removed, vec![2, 3]);
        // A freed id can be used again.
        assert!(commit(&mut t, &[create(2, ControlKind::Text)]).is_empty());
    }

    #[test]
    fn props_text_and_listeners() {
        let mut t = Tree::new();
        commit(
            &mut t,
            &[
                create(1, ControlKind::Button),
                Op::SetProp { id: 1, prop: prop::LABEL, value: "Go".into() },
                Op::SetProp { id: 1, prop: prop::LABEL, value: "Stop".into() },
                Op::Listen { id: 1, event: event::PRESS, handler: 5 },
                Op::Text { id: 1, value: "x".into() },
            ],
        );
        let n = t.get(1).unwrap();
        assert_eq!(n.str_prop(prop::LABEL), Some("Stop"));
        assert_eq!(n.handler(event::PRESS), Some(5));
        assert_eq!(n.text.as_deref(), Some("x"));
        commit(&mut t, &[Op::Unlisten { id: 1, event: event::PRESS }]);
        assert_eq!(t.get(1).unwrap().handler(event::PRESS), None);
    }

    #[test]
    fn bad_ops_are_skipped_not_fatal() {
        let mut t = Tree::new();
        let errors = commit(
            &mut t,
            &[
                create(1, ControlKind::Screen),
                create(1, ControlKind::Screen),
                insert(1, 99, 0),
                insert(1, 1, 0),
                create(0, ControlKind::Text),
                create(2, ControlKind::Text),
            ],
        );
        assert_eq!(errors.iter().map(|e| e.index).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert!(t.get(2).is_some());
    }

    #[test]
    fn cycles_are_rejected() {
        let mut t = Tree::new();
        commit(&mut t, &[create(1, ControlKind::Section), create(2, ControlKind::Section), insert(1, 2, 0)]);
        let errors = commit(&mut t, &[insert(2, 1, 0)]);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn malformed_commit_changes_nothing() {
        let mut t = Tree::new();
        let mut w = Writer::new();
        w.op(&create(1, ControlKind::Screen));
        let mut bytes = w.take();
        bytes.push(0xEE);
        assert!(t.apply(&bytes).is_err());
        assert!(t.is_empty());
    }

    #[test]
    fn unknown_kind_is_kept_as_placeholder() {
        let mut t = Tree::new();
        assert!(commit(&mut t, &[Op::Create { id: 1, kind: 999 }]).is_empty());
        let n = t.get(1).unwrap();
        assert_eq!((n.kind, n.raw_kind), (None, 999));
    }
}
