//! The Plinth guest/host wire format (SPEC.md §8.2 and §8.4).
//!
//! Both sides use this crate: the guest runtime writes ops and reads events,
//! and the host reads ops and writes events. All integers are little-endian.
//!
//! Without the default `std` feature the crate is `no_std` (with `alloc`),
//! so the guest runtime can use it.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

mod ids {
    include!(concat!(env!("OUT_DIR"), "/ids.rs"));
}

pub use ids::*;

use core::fmt;

/// A guest-assigned node id. `0` is the "no node" value.
pub type NodeId = u32;

/// Op codes (SPEC.md §8.2).
pub mod opcode {
    pub const CREATE: u8 = 0x01;
    pub const REMOVE: u8 = 0x02;
    pub const INSERT: u8 = 0x03;
    pub const MOVE: u8 = 0x04;
    pub const SET_PROP: u8 = 0x05;
    pub const LISTEN: u8 = 0x06;
    pub const UNLISTEN: u8 = 0x07;
    pub const SET_ROOT: u8 = 0x08;
    pub const NAVIGATE: u8 = 0x09;
    pub const TEXT: u8 = 0x0A;
    /// Dev-only (SPEC.md §13): carries a hot-reload signal snapshot back to
    /// the host. Only a dev build of `plinth-rt` ever emits this.
    pub const SNAPSHOT: u8 = 0x0B;
}

/// Event codes (SPEC.md §8.4).
pub mod event_code {
    pub const UI: u8 = 0x01;
    pub const COMPLETION: u8 = 0x02;
    pub const TIMER: u8 = 0x03;
    pub const LIFECYCLE: u8 = 0x04;
    pub const VISIBLE_ROWS: u8 = 0x05;
    /// Dev-only (SPEC.md §13): asks a dev build to serialize its preserved
    /// module-level signals. The guest replies with an `Op::Snapshot` in
    /// its next commit.
    pub const SNAPSHOT_REQUEST: u8 = 0x06;
}

/// `kind` values of the `navigate` op.
/// The `init` args buffer (SPEC.md §8.1): a sequence of records, each a
/// `tag: u8`, a `len: u32` and `len` bytes. An empty buffer is a normal
/// start. A guest skips records with tags that it does not know.
pub mod init_arg {
    use alloc::vec::Vec;

    /// Collect after every `init` and `on-event` and poison freed memory
    /// (tests only, SPEC.md §16). No data.
    pub const GC_STRESS: u8 = 1;
    /// A hot-reload signal snapshot from the previous instance (dev only,
    /// SPEC.md §13).
    pub const SNAPSHOT: u8 = 2;

    /// Appends one record.
    pub fn push(out: &mut Vec<u8>, tag: u8, data: &[u8]) {
        out.push(tag);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }

    /// One record as a buffer.
    pub fn one(tag: u8, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        push(&mut out, tag, data);
        out
    }

    /// The data of the first record with `tag`. A malformed buffer gives
    /// `None`.
    pub fn find(buf: &[u8], tag: u8) -> Option<&[u8]> {
        let mut rest = buf;
        while rest.len() >= 5 {
            let len = u32::from_le_bytes([rest[1], rest[2], rest[3], rest[4]]) as usize;
            let data = rest.get(5..5 + len)?;
            if rest[0] == tag {
                return Some(data);
            }
            rest = &rest[5 + len..];
        }
        None
    }
}

pub mod nav_kind {
    pub const PUSH: u8 = 0;
    pub const REPLACE: u8 = 1;
    pub const BACK: u8 = 2;
    pub const SELECT_PRIMARY: u8 = 3;
    /// Declares `screen` as one of the top-level (primary) destinations.
    /// Sent once per primary screen, before any `select-primary` (UI API 1.2).
    pub const MARK_PRIMARY: u8 = 4;
}

/// A dynamically typed value on the wire.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i32),
    Number(f64),
    Str(String),
    Enum(u16),
    List(Vec<Value>),
    Handle(u32),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Number(n) => Some(*n as i32),
            _ => None,
        }
    }

    pub fn as_enum(&self) -> Option<u16> {
        match self {
            Value::Enum(e) => Some(*e),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(v.to_owned())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}

mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 2;
    pub const NUMBER: u8 = 3;
    pub const STRING: u8 = 4;
    pub const ENUM: u8 = 5;
    pub const LIST: u8 = 6;
    pub const HANDLE: u8 = 7;
}

/// One decoded op.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Create { id: NodeId, kind: u16 },
    Remove { id: NodeId },
    Insert { parent: NodeId, id: NodeId, before: NodeId },
    Move { parent: NodeId, id: NodeId, before: NodeId },
    SetProp { id: NodeId, prop: u16, value: Value },
    Listen { id: NodeId, event: u16, handler: u32 },
    Unlisten { id: NodeId, event: u16 },
    SetRoot { screen: u32, id: NodeId },
    Navigate { kind: u8, screen: u32, args: Value },
    Text { id: NodeId, value: Value },
    /// Dev-only (SPEC.md §13): a hot-reload signal snapshot, sent in
    /// response to `Event::SnapshotRequest`.
    Snapshot { bytes: Vec<u8> },
}

/// One decoded event.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Ui { handler: u32, event: u16, value: Value },
    Completion { request: u32, result: Value },
    Timer { timer: u32 },
    Lifecycle { kind: u8 },
    VisibleRows { list: u32, from: u32, to: u32 },
    /// Dev-only (SPEC.md §13): asks for a hot-reload signal snapshot.
    SnapshotRequest,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    pub offset: usize,
    pub message: &'static str,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}

// ---------------------------------------------------------------------------
// Writing

/// An append-only byte buffer with typed writers.
#[derive(Default, Clone, Debug)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub const fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Returns the bytes and leaves the writer empty.
    pub fn take(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.buf)
    }

    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn value(&mut self, v: &Value) {
        match v {
            Value::Null => self.u8(tag::NULL),
            Value::Bool(b) => {
                self.u8(tag::BOOL);
                self.u8(*b as u8);
            }
            Value::Int(i) => {
                self.u8(tag::INT);
                self.buf.extend_from_slice(&i.to_le_bytes());
            }
            Value::Number(n) => {
                self.u8(tag::NUMBER);
                self.buf.extend_from_slice(&n.to_le_bytes());
            }
            Value::Str(s) => {
                self.u8(tag::STRING);
                self.u32(s.len() as u32);
                self.buf.extend_from_slice(s.as_bytes());
            }
            Value::Enum(e) => {
                self.u8(tag::ENUM);
                self.u16(*e);
            }
            Value::List(items) => {
                self.u8(tag::LIST);
                self.u32(items.len() as u32);
                for item in items {
                    self.value(item);
                }
            }
            Value::Handle(h) => {
                self.u8(tag::HANDLE);
                self.u32(*h);
            }
        }
    }

    pub fn op(&mut self, op: &Op) {
        match op {
            Op::Create { id, kind } => {
                self.u8(opcode::CREATE);
                self.u32(*id);
                self.u16(*kind);
            }
            Op::Remove { id } => {
                self.u8(opcode::REMOVE);
                self.u32(*id);
            }
            Op::Insert { parent, id, before } => {
                self.u8(opcode::INSERT);
                self.u32(*parent);
                self.u32(*id);
                self.u32(*before);
            }
            Op::Move { parent, id, before } => {
                self.u8(opcode::MOVE);
                self.u32(*parent);
                self.u32(*id);
                self.u32(*before);
            }
            Op::SetProp { id, prop, value } => {
                self.u8(opcode::SET_PROP);
                self.u32(*id);
                self.u16(*prop);
                self.value(value);
            }
            Op::Listen { id, event, handler } => {
                self.u8(opcode::LISTEN);
                self.u32(*id);
                self.u16(*event);
                self.u32(*handler);
            }
            Op::Unlisten { id, event } => {
                self.u8(opcode::UNLISTEN);
                self.u32(*id);
                self.u16(*event);
            }
            Op::SetRoot { screen, id } => {
                self.u8(opcode::SET_ROOT);
                self.u32(*screen);
                self.u32(*id);
            }
            Op::Navigate { kind, screen, args } => {
                self.u8(opcode::NAVIGATE);
                self.u8(*kind);
                self.u32(*screen);
                self.value(args);
            }
            Op::Text { id, value } => {
                self.u8(opcode::TEXT);
                self.u32(*id);
                self.value(value);
            }
            Op::Snapshot { bytes } => {
                self.u8(opcode::SNAPSHOT);
                self.u32(bytes.len() as u32);
                self.buf.extend_from_slice(bytes);
            }
        }
    }

    pub fn event(&mut self, ev: &Event) {
        match ev {
            Event::Ui { handler, event, value } => {
                self.u8(event_code::UI);
                self.u32(*handler);
                self.u16(*event);
                self.value(value);
            }
            Event::Completion { request, result } => {
                self.u8(event_code::COMPLETION);
                self.u32(*request);
                self.value(result);
            }
            Event::Timer { timer } => {
                self.u8(event_code::TIMER);
                self.u32(*timer);
            }
            Event::Lifecycle { kind } => {
                self.u8(event_code::LIFECYCLE);
                self.u8(*kind);
            }
            Event::VisibleRows { list, from, to } => {
                self.u8(event_code::VISIBLE_ROWS);
                self.u32(*list);
                self.u32(*from);
                self.u32(*to);
            }
            Event::SnapshotRequest => {
                self.u8(event_code::SNAPSHOT_REQUEST);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Reading

/// A cursor over a byte buffer with typed readers.
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

/// The maximum nesting depth of `Value::List`. It stops stack overflows on
/// hostile input.
const MAX_VALUE_DEPTH: u32 = 32;

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn err(&self, message: &'static str) -> DecodeError {
        DecodeError { offset: self.pos, message }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        let end = end.ok_or_else(|| self.err("unexpected end of buffer"))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn value(&mut self) -> Result<Value, DecodeError> {
        self.value_at_depth(0)
    }

    fn value_at_depth(&mut self, depth: u32) -> Result<Value, DecodeError> {
        let start = self.pos;
        Ok(match self.u8()? {
            tag::NULL => Value::Null,
            tag::BOOL => Value::Bool(self.u8()? != 0),
            tag::INT => Value::Int(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap())),
            tag::NUMBER => Value::Number(f64::from_le_bytes(self.bytes(8)?.try_into().unwrap())),
            tag::STRING => {
                let len = self.u32()? as usize;
                let bytes = self.bytes(len)?;
                let s = core::str::from_utf8(bytes)
                    .map_err(|_| DecodeError { offset: start, message: "string is not UTF-8" })?;
                Value::Str(s.to_owned())
            }
            tag::ENUM => Value::Enum(self.u16()?),
            tag::LIST => {
                if depth >= MAX_VALUE_DEPTH {
                    return Err(DecodeError { offset: start, message: "list nesting too deep" });
                }
                let count = self.u32()? as usize;
                // Each item takes at least one byte, so a larger count is malformed.
                if count > self.buf.len() - self.pos {
                    return Err(self.err("list count exceeds buffer"));
                }
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    items.push(self.value_at_depth(depth + 1)?);
                }
                Value::List(items)
            }
            tag::HANDLE => Value::Handle(self.u32()?),
            _ => return Err(DecodeError { offset: start, message: "unknown value tag" }),
        })
    }

    pub fn op(&mut self) -> Result<Op, DecodeError> {
        let start = self.pos;
        Ok(match self.u8()? {
            opcode::CREATE => Op::Create { id: self.u32()?, kind: self.u16()? },
            opcode::REMOVE => Op::Remove { id: self.u32()? },
            opcode::INSERT => Op::Insert { parent: self.u32()?, id: self.u32()?, before: self.u32()? },
            opcode::MOVE => Op::Move { parent: self.u32()?, id: self.u32()?, before: self.u32()? },
            opcode::SET_PROP => Op::SetProp { id: self.u32()?, prop: self.u16()?, value: self.value()? },
            opcode::LISTEN => Op::Listen { id: self.u32()?, event: self.u16()?, handler: self.u32()? },
            opcode::UNLISTEN => Op::Unlisten { id: self.u32()?, event: self.u16()? },
            opcode::SET_ROOT => Op::SetRoot { screen: self.u32()?, id: self.u32()? },
            opcode::NAVIGATE => Op::Navigate { kind: self.u8()?, screen: self.u32()?, args: self.value()? },
            opcode::TEXT => Op::Text { id: self.u32()?, value: self.value()? },
            opcode::SNAPSHOT => {
                let len = self.u32()? as usize;
                Op::Snapshot { bytes: self.bytes(len)?.to_vec() }
            }
            _ => return Err(DecodeError { offset: start, message: "unknown opcode" }),
        })
    }

    pub fn event(&mut self) -> Result<Event, DecodeError> {
        let start = self.pos;
        Ok(match self.u8()? {
            event_code::UI => Event::Ui { handler: self.u32()?, event: self.u16()?, value: self.value()? },
            event_code::COMPLETION => Event::Completion { request: self.u32()?, result: self.value()? },
            event_code::TIMER => Event::Timer { timer: self.u32()? },
            event_code::LIFECYCLE => Event::Lifecycle { kind: self.u8()? },
            event_code::VISIBLE_ROWS => {
                Event::VisibleRows { list: self.u32()?, from: self.u32()?, to: self.u32()? }
            }
            event_code::SNAPSHOT_REQUEST => Event::SnapshotRequest,
            _ => return Err(DecodeError { offset: start, message: "unknown event code" }),
        })
    }
}

/// Decodes a complete op buffer. A malformed buffer gives an error and no ops.
pub fn decode_ops(buf: &[u8]) -> Result<Vec<Op>, DecodeError> {
    let mut r = Reader::new(buf);
    let mut ops = Vec::new();
    while !r.is_empty() {
        ops.push(r.op()?);
    }
    Ok(ops)
}

/// Decodes a complete event buffer.
pub fn decode_events(buf: &[u8]) -> Result<Vec<Event>, DecodeError> {
    let mut r = Reader::new(buf);
    let mut events = Vec::new();
    while !r.is_empty() {
        events.push(r.event()?);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn ops_round_trip() {
        let ops = vec![
            Op::Create { id: 1, kind: ControlKind::Screen as u16 },
            Op::SetProp { id: 1, prop: prop::TITLE, value: "Home".into() },
            Op::Create { id: 2, kind: ControlKind::Button as u16 },
            Op::Insert { parent: 1, id: 2, before: 0 },
            Op::Listen { id: 2, event: event::PRESS, handler: 7 },
            Op::SetProp {
                id: 2,
                prop: prop::VALUE,
                value: Value::List(vec![Value::Null, Value::Bool(true), Value::Number(1.5), Value::Handle(3)]),
            },
            Op::Text { id: 2, value: "héllo".into() },
            Op::SetRoot { screen: 0, id: 1 },
            Op::Navigate { kind: nav_kind::SELECT_PRIMARY, screen: 0, args: Value::Null },
            Op::Move { parent: 1, id: 2, before: 0 },
            Op::Unlisten { id: 2, event: event::PRESS },
            Op::Remove { id: 2 },
            Op::Snapshot { bytes: vec![1, 2, 3, 0, 255] },
        ];
        let mut w = Writer::new();
        for op in &ops {
            w.op(op);
        }
        assert_eq!(decode_ops(w.as_bytes()).unwrap(), ops);
    }

    #[test]
    fn events_round_trip() {
        let events = vec![
            Event::Ui { handler: 4, event: event::CHANGE, value: Value::Str("abc".into()) },
            Event::Completion { request: 9, result: Value::Int(-3) },
            Event::Timer { timer: 2 },
            Event::Lifecycle { kind: 1 },
            Event::VisibleRows { list: 5, from: 0, to: 20 },
            Event::SnapshotRequest,
        ];
        let mut w = Writer::new();
        for ev in &events {
            w.event(ev);
        }
        assert_eq!(decode_events(w.as_bytes()).unwrap(), events);
    }

    #[test]
    fn truncated_buffer_is_an_error() {
        let mut w = Writer::new();
        w.op(&Op::Create { id: 1, kind: 1 });
        let bytes = w.as_bytes();
        assert!(decode_ops(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn hostile_list_count_is_an_error() {
        let buf = [opcode::TEXT, 1, 0, 0, 0, 6, 0xff, 0xff, 0xff, 0xff];
        assert!(decode_ops(&buf).is_err());
    }

    #[test]
    fn generated_ids() {
        assert_eq!(ControlKind::from_u16(6), Some(ControlKind::TextField));
        assert_eq!(ControlKind::TextField.name(), "text-field");
        assert_eq!(prop::name(prop::VALUE), Some("value"));
        assert_eq!(button_role::DESTRUCTIVE, 2);
        assert_eq!(UI_API_VERSION, "1.10");
    }
}
