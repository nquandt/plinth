//! A hostile guest: the op stream comes from app code that the host does
//! not trust (any `.plnt` can send any bytes through `ui.commit`). The tree
//! applier must never panic or hang, and after every commit the tree must
//! hold its invariants (each child points back to its parent, no node is in
//! two places, no cycle), because the renderer walks it recursively.
//!
//! Two kinds of input: random well-formed ops over a small id range (bad
//! parents, cycles, removed nodes, wrong value types), and the byte
//! encoding of such ops with random changes. The random source is fixed.
//! `PLINTH_FUZZ_ITERS` sets the number of commits (default 3000).

use plinth_protocol::{NodeId, Op, Value, Writer, nav_kind};
use plinth_ui::tree::Tree;
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    fn id(&mut self) -> NodeId {
        // Mostly a small range, so ops hit live nodes; sometimes 0 or huge.
        match self.below(20) {
            0 => 0,
            1 => u32::MAX,
            2 => plinth_ui::tree::MAX_NODE_ID,
            _ => 1 + self.below(24) as NodeId,
        }
    }
    fn value(&mut self, depth: u32) -> Value {
        match self.below(if depth > 2 { 8 } else { 9 }) {
            0 => Value::Null,
            1 => Value::Bool(self.below(2) == 1),
            2 => Value::Int(self.next() as i32),
            3 => Value::Number([0.0, -1.0, 1e308, f64::NAN, f64::INFINITY, -0.5, 12.25][self.below(7) as usize]),
            4 => Value::Str(["", "a", "x\u{0}y", "\u{1F600}", "#zz", "1/2", "full", &"w".repeat(5000)][self.below(8) as usize].to_owned()),
            5 => Value::Enum(self.below(70) as u16),
            6 => Value::Handle(self.next() as u32),
            7 => Value::Str(format!("R,{},{},{}", self.below(9), self.below(9), self.below(9))),
            _ => Value::List((0..self.below(5)).map(|_| self.value(depth + 1)).collect()),
        }
    }
    fn op(&mut self) -> Op {
        match self.below(12) {
            0 | 1 => Op::Create { id: self.id(), kind: self.below(70) as u16 },
            2 => Op::Remove { id: self.id() },
            3 | 4 => Op::Insert { parent: self.id(), id: self.id(), before: self.id() },
            5 => Op::Move { parent: self.id(), id: self.id(), before: self.id() },
            6 | 7 => Op::SetProp { id: self.id(), prop: self.below(120) as u16, value: self.value(0) },
            8 => Op::Listen { id: self.id(), event: self.below(30) as u16, handler: self.next() as u32 },
            9 => Op::Text { id: self.id(), value: self.value(1) },
            10 => Op::SetRoot { screen: self.below(4) as u32, id: self.id() },
            _ => Op::Navigate {
                kind: [nav_kind::SELECT_PRIMARY, nav_kind::REPLACE, nav_kind::MARK_PRIMARY, nav_kind::PUSH, nav_kind::BACK, 99][self.below(6) as usize],
                screen: self.below(6) as u32,
                args: Value::Null,
            },
        }
    }
}

/// Checks the tree's invariants; returns the first problem.
fn check(tree: &Tree) -> Result<(), String> {
    let mut placed: HashSet<NodeId> = HashSet::new();
    for id in 1..=64u32 {
        let Some(n) = tree.get(id) else { continue };
        for &c in &n.children {
            let child = tree.get(c).ok_or_else(|| format!("node {id} has a removed child {c}"))?;
            if child.parent != id {
                return Err(format!("child {c} of {id} names parent {}", child.parent));
            }
            if !placed.insert(c) {
                return Err(format!("node {c} is a child two times"));
            }
        }
        if n.parent != 0 && !tree.get(n.parent).is_some_and(|p| p.children.contains(&id)) {
            return Err(format!("node {id} names parent {}, which does not hold it", n.parent));
        }
        // No cycle: the parent chain ends.
        let (mut cur, mut steps) = (n.parent, 0);
        while cur != 0 {
            steps += 1;
            if steps > 100 || cur == id {
                return Err(format!("node {id} is in a cycle"));
            }
            cur = tree.get(cur).map_or(0, |p| p.parent);
        }
    }
    Ok(())
}

fn encode(ops: &[Op]) -> Vec<u8> {
    let mut w = Writer::new();
    for op in ops {
        w.op(op);
    }
    w.as_bytes().to_vec()
}

fn mutate(bytes: &mut Vec<u8>, rng: &mut Rng) {
    for _ in 0..1 + rng.below(4) {
        if bytes.is_empty() {
            bytes.push(rng.next() as u8);
            continue;
        }
        let at = rng.below(bytes.len() as u64) as usize;
        match rng.below(5) {
            0 => bytes[at] = rng.next() as u8,
            1 => bytes[at] ^= 1 << rng.below(8),
            2 => bytes.truncate(at),
            3 => bytes.insert(at, [0x00, 0xFF, 0x7F, 0x80][rng.below(4) as usize]),
            _ => {
                let end = (at + 1 + rng.below(16) as usize).min(bytes.len());
                let piece = bytes[at..end].to_vec();
                bytes.splice(at..at, piece);
            }
        }
    }
}

#[test]
fn hostile_op_streams_never_panic_and_keep_the_tree_valid() {
    let iters: usize = std::env::var("PLINTH_FUZZ_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(3000);
    let mut rng = Rng(0x0B5E);
    let mut tree = Tree::new();
    let (mut applied, mut rejected) = (0, 0);
    for commit in 0..iters {
        // Start a new tree now and then, so both young and old trees get ops.
        if rng.below(50) == 0 {
            tree = Tree::new();
        }
        let ops: Vec<Op> = (0..1 + rng.below(30)).map(|_| rng.op()).collect();
        let mut bytes = encode(&ops);
        if rng.below(3) == 0 {
            mutate(&mut bytes, &mut rng);
        }
        let result = catch_unwind(AssertUnwindSafe(|| tree.apply(&bytes)));
        match result {
            Ok(Ok(_)) => applied += 1,
            Ok(Err(_)) => rejected += 1,
            Err(_) => panic!("commit {commit} panicked; ops {ops:?}, bytes {bytes:?}"),
        }
        if let Err(problem) = check(&tree) {
            panic!("commit {commit} broke the tree: {problem}; ops {ops:?}");
        }
        // The views that the renderer reads must not panic either.
        let _ = tree.screens().count();
        let _ = tree.primary_screens().count();
        let _ = tree.take_removed();
    }
    eprintln!("{iters} commits: {applied} applied, {rejected} rejected as malformed");
    assert!(applied > iters / 2 && rejected > 0, "the test must reach both paths");
}
