// A tiny read-only zip reader for `.plnt` packages (SPEC.md §18.3): just
// enough to find named entries in the central directory and inflate them.
// No npm dependency: uses the standard `DecompressionStream("deflate-raw")`,
// which both modern browsers and Node 18+ provide globally.

function readU32(view, off) {
  return view.getUint32(off, true);
}
function readU16(view, off) {
  return view.getUint16(off, true);
}

/** Finds the End Of Central Directory record. Assumes no zip comment trickery. */
function findEocd(bytes) {
  const sig = 0x06054b50;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let i = bytes.length - 22; i >= 0; i--) {
    if (readU32(view, i) === sig) return i;
  }
  throw new Error("not a zip file (no End Of Central Directory record)");
}

/** Parses the central directory into `{name -> entry}`, entry has method/sizes/offset. */
function readCentralDirectory(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const eocd = findEocd(bytes);
  const count = readU16(view, eocd + 10);
  let off = readU32(view, eocd + 16);
  const entries = new Map();
  for (let i = 0; i < count; i++) {
    if (readU32(view, off) !== 0x02014b50) throw new Error("malformed central directory entry");
    const method = readU16(view, off + 10);
    const compSize = readU32(view, off + 20);
    const uncompSize = readU32(view, off + 24);
    const nameLen = readU16(view, off + 28);
    const extraLen = readU16(view, off + 30);
    const commentLen = readU16(view, off + 32);
    const localHeaderOffset = readU32(view, off + 42);
    const name = new TextDecoder().decode(bytes.subarray(off + 46, off + 46 + nameLen));
    entries.set(name, { method, compSize, uncompSize, localHeaderOffset });
    off += 46 + nameLen + extraLen + commentLen;
  }
  return entries;
}

/** Reads the raw (still compressed) bytes of one local file entry. */
function readLocalData(bytes, entry) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const off = entry.localHeaderOffset;
  if (readU32(view, off) !== 0x04034b50) throw new Error("malformed local file header");
  const nameLen = readU16(view, off + 26);
  const extraLen = readU16(view, off + 28);
  const dataStart = off + 30 + nameLen + extraLen;
  return bytes.subarray(dataStart, dataStart + entry.compSize);
}

async function inflateRaw(compressed) {
  const stream = new Blob([compressed]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  const buf = await new Response(stream).arrayBuffer();
  return new Uint8Array(buf);
}

/**
 * Reads a `.plnt` zip. Returns an object with `names()` and
 * `read(name) -> Promise<Uint8Array>`.
 */
export function openZip(bytes) {
  const entries = readCentralDirectory(bytes);
  return {
    names() {
      return [...entries.keys()];
    },
    has(name) {
      return entries.has(name);
    },
    async read(name) {
      const entry = entries.get(name);
      if (!entry) throw new Error(`the package has no entry \`${name}\``);
      const raw = readLocalData(bytes, entry);
      if (entry.method === 0) return raw; // stored
      if (entry.method === 8) return inflateRaw(raw); // deflated
      throw new Error(`entry \`${name}\` uses unsupported zip compression method ${entry.method}`);
    },
  };
}
