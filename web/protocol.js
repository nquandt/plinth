// The Plinth guest/host wire format (SPEC.md §8.2, §8.4), decoder/encoder
// for JS hosts (browser and Node). Mirrors crates/plinth-protocol/src/lib.rs
// byte for byte; keep the two in sync by hand (no codegen for this part).

export const OpCode = {
  CREATE: 0x01,
  REMOVE: 0x02,
  INSERT: 0x03,
  MOVE: 0x04,
  SET_PROP: 0x05,
  LISTEN: 0x06,
  UNLISTEN: 0x07,
  SET_ROOT: 0x08,
  NAVIGATE: 0x09,
  TEXT: 0x0a,
  SNAPSHOT: 0x0b,
};

export const EventCode = {
  UI: 0x01,
  COMPLETION: 0x02,
  TIMER: 0x03,
  LIFECYCLE: 0x04,
  VISIBLE_ROWS: 0x05,
  SNAPSHOT_REQUEST: 0x06,
};

export const NavKind = {
  PUSH: 0,
  REPLACE: 1,
  BACK: 2,
  SELECT_PRIMARY: 3,
  MARK_PRIMARY: 4,
};

const ValueTag = {
  NULL: 0,
  BOOL: 1,
  INT: 2,
  NUMBER: 3,
  STRING: 4,
  ENUM: 5,
  LIST: 6,
  HANDLE: 7,
};

/** A little cursor over a Uint8Array, little-endian, matching `Reader`. */
class ByteReader {
  constructor(buf) {
    this.buf = buf;
    this.pos = 0;
  }
  isEmpty() {
    return this.pos >= this.buf.length;
  }
  u8() {
    return this.buf[this.pos++];
  }
  u16() {
    const v = this.buf[this.pos] | (this.buf[this.pos + 1] << 8);
    this.pos += 2;
    return v;
  }
  u32() {
    const v =
      this.buf[this.pos] |
      (this.buf[this.pos + 1] << 8) |
      (this.buf[this.pos + 2] << 16) |
      (this.buf[this.pos + 3] << 24);
    this.pos += 4;
    return v >>> 0;
  }
  f64() {
    const v = new DataView(this.buf.buffer, this.buf.byteOffset + this.pos, 8).getFloat64(0, true);
    this.pos += 8;
    return v;
  }
  bytes(n) {
    const s = this.buf.subarray(this.pos, this.pos + n);
    this.pos += n;
    return s;
  }
  string() {
    const len = this.u32();
    const bytes = this.bytes(len);
    return new TextDecoder().decode(bytes);
  }
  value() {
    const tag = this.u8();
    switch (tag) {
      case ValueTag.NULL:
        return null;
      case ValueTag.BOOL:
        return this.u8() !== 0;
      case ValueTag.INT: {
        const v = new DataView(this.buf.buffer, this.buf.byteOffset + this.pos, 4).getInt32(0, true);
        this.pos += 4;
        return v;
      }
      case ValueTag.NUMBER:
        return this.f64();
      case ValueTag.STRING:
        return this.string();
      case ValueTag.ENUM:
        return { enum: this.u16() };
      case ValueTag.LIST: {
        const count = this.u32();
        const items = [];
        for (let i = 0; i < count; i++) items.push(this.value());
        return items;
      }
      case ValueTag.HANDLE:
        return { handle: this.u32() };
      default:
        throw new Error(`unknown value tag ${tag} at byte ${this.pos - 1}`);
    }
  }
}

/** Decodes one committed op buffer (`ui.commit`'s argument) into a list of ops. */
export function decodeOps(buf) {
  const r = new ByteReader(buf);
  const ops = [];
  while (!r.isEmpty()) {
    const start = r.pos;
    const op = r.u8();
    switch (op) {
      case OpCode.CREATE:
        ops.push({ op: "create", id: r.u32(), kind: r.u16() });
        break;
      case OpCode.REMOVE:
        ops.push({ op: "remove", id: r.u32() });
        break;
      case OpCode.INSERT:
        ops.push({ op: "insert", parent: r.u32(), id: r.u32(), before: r.u32() });
        break;
      case OpCode.MOVE:
        ops.push({ op: "move", parent: r.u32(), id: r.u32(), before: r.u32() });
        break;
      case OpCode.SET_PROP:
        ops.push({ op: "set-prop", id: r.u32(), prop: r.u16(), value: r.value() });
        break;
      case OpCode.LISTEN:
        ops.push({ op: "listen", id: r.u32(), event: r.u16(), handler: r.u32() });
        break;
      case OpCode.UNLISTEN:
        ops.push({ op: "unlisten", id: r.u32(), event: r.u16() });
        break;
      case OpCode.SET_ROOT:
        ops.push({ op: "set-root", screen: r.u32(), id: r.u32() });
        break;
      case OpCode.NAVIGATE:
        ops.push({ op: "navigate", kind: r.u8(), screen: r.u32(), args: r.value() });
        break;
      case OpCode.TEXT:
        ops.push({ op: "text", id: r.u32(), value: r.value() });
        break;
      case OpCode.SNAPSHOT: {
        const len = r.u32();
        ops.push({ op: "snapshot", bytes: r.bytes(len) });
        break;
      }
      default:
        throw new Error(`unknown opcode ${op} at byte ${start}`);
    }
  }
  return ops;
}

/** A little append-only byte writer, little-endian, matching `Writer`. */
class ByteWriter {
  constructor() {
    this.chunks = [];
  }
  u8(v) {
    this.chunks.push(Uint8Array.of(v & 0xff));
  }
  u16(v) {
    this.chunks.push(Uint8Array.of(v & 0xff, (v >> 8) & 0xff));
  }
  u32(v) {
    const b = new Uint8Array(4);
    new DataView(b.buffer).setUint32(0, v >>> 0, true);
    this.chunks.push(b);
  }
  f64(v) {
    const b = new Uint8Array(8);
    new DataView(b.buffer).setFloat64(0, v, true);
    this.chunks.push(b);
  }
  bytes(b) {
    this.chunks.push(b);
  }
  string(s) {
    const bytes = new TextEncoder().encode(s);
    this.u32(bytes.length);
    this.bytes(bytes);
  }
  value(v) {
    if (v === null || v === undefined) {
      this.u8(ValueTag.NULL);
    } else if (typeof v === "boolean") {
      this.u8(ValueTag.BOOL);
      this.u8(v ? 1 : 0);
    } else if (typeof v === "string") {
      this.u8(ValueTag.STRING);
      this.string(v);
    } else if (typeof v === "number") {
      // JS hosts always write a Number; integer-typed props accept it too.
      this.u8(ValueTag.NUMBER);
      this.f64(v);
    } else if (v && typeof v === "object" && "enum" in v) {
      this.u8(ValueTag.ENUM);
      this.u16(v.enum);
    } else if (v && typeof v === "object" && "handle" in v) {
      this.u8(ValueTag.HANDLE);
      this.u32(v.handle);
    } else if (Array.isArray(v)) {
      this.u8(ValueTag.LIST);
      this.u32(v.length);
      for (const item of v) this.value(item);
    } else {
      throw new Error(`cannot encode value ${JSON.stringify(v)}`);
    }
  }
  finish() {
    let len = 0;
    for (const c of this.chunks) len += c.length;
    const out = new Uint8Array(len);
    let off = 0;
    for (const c of this.chunks) {
      out.set(c, off);
      off += c.length;
    }
    return out;
  }
}

/** Encodes one event. `event` is `{kind: "ui"|"timer"|"lifecycle"|"visible-rows"|"snapshot-request", ...}`. */
export function encodeEvent(event) {
  const w = new ByteWriter();
  switch (event.kind) {
    case "ui":
      w.u8(EventCode.UI);
      w.u32(event.handler);
      w.u16(event.event);
      w.value(event.value ?? null);
      break;
    case "completion":
      w.u8(EventCode.COMPLETION);
      w.u32(event.request);
      w.value(event.result ?? null);
      break;
    case "timer":
      w.u8(EventCode.TIMER);
      w.u32(event.timer);
      break;
    case "lifecycle":
      w.u8(EventCode.LIFECYCLE);
      w.u8(event.lifecycleKind);
      break;
    case "visible-rows":
      w.u8(EventCode.VISIBLE_ROWS);
      w.u32(event.list);
      w.u32(event.from);
      w.u32(event.to);
      break;
    case "snapshot-request":
      w.u8(EventCode.SNAPSHOT_REQUEST);
      break;
    default:
      throw new Error(`unknown event kind ${event.kind}`);
  }
  return w.finish();
}

/** Encodes an `init` args buffer: a sequence of `(tag: u8, len: u32, bytes)` records (SPEC.md §8.1). */
export function encodeInitArgs(records) {
  const w = new ByteWriter();
  for (const { tag, data } of records) {
    w.u8(tag);
    w.u32(data.length);
    w.bytes(data);
  }
  return w.finish();
}
