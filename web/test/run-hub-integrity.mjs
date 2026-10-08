// Node test (docs/web-hub.md §4.3, docs/REGISTRY.md §5): the package checks
// of the web App Hub (`hub-integrity.js`) against packages that the
// `plinth` CLI made and signed, and against a registry that
// `plinth registry build` wrote. Node 22 has WebCrypto with Ed25519, as
// new browsers do.
//
// Build first: cargo build -p plinth-cli; target/debug/plinth build examples/counter examples/notes

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { sha256Hex, checkPackage, checkCore, canonicalJson, ed25519Supported } from "../hub-integrity.js";
import { openZip } from "../zip.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const exe = path.join(root, "target/debug", process.platform === "win32" ? "plinth.exe" : "plinth");

// -- A tiny zip writer (stored entries), to change one entry of a package --

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}
function writeZip(entries) {
  const enc = new TextEncoder();
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const [name, data] of entries) {
    const n = enc.encode(name);
    const crc = crc32(data);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(n.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(n.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, n, data);
    centrals.push(central, n);
    offset += 30 + n.length + data.length;
  }
  const cd = Buffer.concat(centrals);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(offset, 16);
  return new Uint8Array(Buffer.concat([...locals, cd, end]));
}
async function rezip(bytes, change) {
  const zip = openZip(bytes);
  const entries = [];
  for (const name of zip.names()) entries.push([name, Buffer.from(await zip.read(name))]);
  return writeZip(change(entries) ?? entries);
}

async function rejects(promise, pattern, what) {
  await assert.rejects(promise, pattern, what);
}

async function main() {
  assert.equal(await sha256Hex(new TextEncoder().encode("abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
  assert.equal(canonicalJson({ b: 1, a: { d: [1, "x"], c: null } }), '{"a":{"c":null,"d":[1,"x"]},"b":1}');
  assert.equal(await ed25519Supported(), true, "Node 22 WebCrypto has Ed25519");

  const dir = mkdtempSync(path.join(tmpdir(), "plinth-web-integrity-"));
  try {
    // A registry with an unsigned counter and a signed notes.
    const env = { ...process.env, PLINTH_PUBLISHER_DIR: path.join(dir, "publisher") };
    execFileSync(exe, ["publisher", "init", "--name", "plinth"], { env, stdio: "ignore" });
    const reg = path.join(dir, "registry");
    mkdirSync(reg);
    copyFileSync(path.join(root, "examples/counter/dist/counter.plnt"), path.join(reg, "counter.plnt"));
    copyFileSync(path.join(root, "examples/notes/dist/notes.plnt"), path.join(reg, "notes.plnt"));
    execFileSync(exe, ["sign", path.join(reg, "notes.plnt")], { env, stdio: "ignore" });
    execFileSync(exe, ["registry", "build", reg, "--with-core"], { stdio: "ignore" });

    const doc = (id) => JSON.parse(readFileSync(path.join(reg, "apps", id, "index.json"), "utf8"));
    const pkg = (v) => new Uint8Array(readFileSync(path.join(reg, "packages", `${v.sha256}.plnt`)));
    const counterV = doc("dev.plinth.examples.counter").versions[0];
    const notesV = doc("dev.plinth.examples.notes").versions[0];
    assert.ok(notesV.signer?.startsWith("ed25519:"), "the registry records the signer");
    assert.equal(counterV.signer, undefined);

    // Unsigned: the digest and the manifest are checked.
    const counter = await checkPackage(pkg(counterV), counterV, "dev.plinth.examples.counter");
    assert.equal(counter.signature.status, "unsigned");
    assert.equal(counter.manifest.id, "dev.plinth.examples.counter");
    assert.match(counter.manifestText, /^id = /m);
    await rejects(checkPackage(pkg(counterV), { ...counterV, sha256: "0".repeat(64) }, "dev.plinth.examples.counter"), /does not match the registry digest/, "digest mismatch");
    await rejects(checkPackage(pkg(counterV), { ...counterV, sha256: "" }, "dev.plinth.examples.counter"), /no SHA-256/);
    await rejects(checkPackage(pkg(counterV), counterV, "dev.plinth.examples.notes"), /not "dev.plinth.examples.notes"/, "another app id");
    await rejects(checkPackage(pkg(counterV), { ...counterV, version: "9.9.9" }, "dev.plinth.examples.counter"), /not 9\.9\.9/, "another version");
    await rejects(checkPackage(pkg(counterV), { ...counterV, signer: notesV.signer }, "dev.plinth.examples.counter"), /has no signature/, "registry says signed");

    // Signed: the Ed25519 signature is checked.
    const notes = await checkPackage(pkg(notesV), notesV, "dev.plinth.examples.notes");
    assert.deepEqual([notes.signature.status, notes.signature.key, notes.signature.publisher], ["verified", notesV.signer, "plinth"]);
    const notChecked = await checkPackage(pkg(notesV), notesV, "dev.plinth.examples.notes", { canVerify: false });
    assert.equal(notChecked.signature.status, "not-checked", "no Ed25519 in this browser: not checked, not an error");
    await rejects(
      checkPackage(pkg(notesV), { ...notesV, signer: "ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA" }, "dev.plinth.examples.notes"),
      /but the registry says/,
      "another signer",
    );

    // A changed signature (the registry digest agrees with the changed file).
    const forged = await rezip(pkg(notesV), (entries) =>
      entries.map(([name, data]) => {
        if (name !== "signature.json") return [name, data];
        const sig = JSON.parse(data.toString("utf8"));
        const raw = Buffer.from(sig.signature, "base64");
        raw[0] ^= 1;
        sig.signature = raw.toString("base64");
        return [name, Buffer.from(JSON.stringify(sig))];
      }),
    );
    const forgedV = { ...notesV, sha256: await sha256Hex(forged) };
    await rejects(checkPackage(forged, forgedV, "dev.plinth.examples.notes"), /does not verify/, "a changed signature");

    // A changed module (the digest list no longer agrees).
    const changed = await rezip(pkg(notesV), (entries) =>
      entries.map(([name, data]) => (name === "app.wasm" ? [name, Buffer.concat([data, Buffer.from([0])])] : [name, data])),
    );
    await rejects(checkPackage(changed, { ...notesV, sha256: await sha256Hex(changed) }, "dev.plinth.examples.notes"), /does not match the manifest|does not match its signature/);

    // A changed manifest (plinth.signature/2 covers manifest.toml). The
    // registry document agrees with each changed file, so only the
    // signature can catch the change.
    const changeManifest = (edit) =>
      rezip(pkg(notesV), (entries) => entries.map(([name, data]) => (name === "manifest.toml" ? [name, Buffer.from(edit(data.toString("utf8")))] : [name, data])));
    const manifestChanges = [
      ["version", (t) => t.replace('version = "0.1.0"', 'version = "9.9.9"'), { version: "9.9.9" }, "dev.plinth.examples.notes"],
      ["id", (t) => t.replace('id = "dev.plinth.examples.notes"', 'id = "dev.plinth.examples.counter"'), {}, "dev.plinth.examples.counter"],
      ["capability", (t) => t.replace('name = "store.kv"', 'name = "net:*"'), {}, "dev.plinth.examples.notes"],
      ["added capability", (t) => `${t}\n[[capabilities]]\nname = "files"\nrationale = "x"\n`, {}, "dev.plinth.examples.notes"],
      ["feed", (t) => `${t}\n[updates]\nfeed = "https://evil.example/feed.json"\n`, {}, "dev.plinth.examples.notes"],
    ];
    for (const [what, edit, extra, appId] of manifestChanges) {
      const bytes = await changeManifest((t) => {
        const out = edit(t);
        assert.notEqual(out, t, `the edit "${what}" changes the manifest`);
        return out;
      });
      const v = { ...notesV, ...extra, sha256: await sha256Hex(bytes) };
      await rejects(checkPackage(bytes, v, appId), /does not match its signature/, `a changed manifest: ${what}`);
    }
    // An old signature schema gets a clear message.
    const oldSchema = await rezip(pkg(notesV), (entries) =>
      entries.map(([name, data]) => {
        if (name !== "signature.json") return [name, data];
        const sig = JSON.parse(data.toString("utf8"));
        sig.schema = "plinth.signature/1";
        return [name, Buffer.from(JSON.stringify(sig))];
      }),
    );
    await rejects(checkPackage(oldSchema, { ...notesV, sha256: await sha256Hex(oldSchema) }, "dev.plinth.examples.notes"), /old format.*plinth sign/, "plinth.signature/1");
    // An added entry is covered too.
    const added = await rezip(pkg(notesV), (entries) => [["extra.txt", Buffer.from("x")], ...entries]);
    await rejects(checkPackage(added, { ...notesV, sha256: await sha256Hex(added) }, "dev.plinth.examples.notes"), /does not match its signature/, "an added entry");
    // Two entries with one name.
    const twice = await rezip(pkg(notesV), (entries) => [...entries, entries.find(([n]) => n === "manifest.toml")]);
    await rejects(checkPackage(twice, { ...notesV, sha256: await sha256Hex(twice) }, "dev.plinth.examples.notes"), /two times/, "a duplicate entry");

    // Format compatibility (docs/COMPATIBILITY.md §4): a newer profile.
    const withProfile = (p) => rezip(pkg(counterV), (entries) => entries.map(([name, data]) => (name === "plinth-profile" ? [name, Buffer.from(p)] : [name, data])));
    for (const p of ["plinth/2", "plinth/17\n"]) {
      const bytes = await withProfile(p);
      await rejects(checkPackage(bytes, { ...counterV, sha256: await sha256Hex(bytes) }, "dev.plinth.examples.counter"), /^Error: This app needs a newer version of Plinth \(package format plinth\/\d+\)$/, p);
    }
    for (const p of ["garbage", "plinth/0", "plinth/x", ""]) {
      const bytes = await withProfile(p);
      await rejects(checkPackage(bytes, { ...counterV, sha256: await sha256Hex(bytes) }, "dev.plinth.examples.counter"), /profile is not valid/, JSON.stringify(p));
    }
    // An unknown entry: unsigned, the package reads; `plinth sign` covers
    // the entry, and the web Hub verifies it.
    const unknown = await rezip(pkg(notesV), (entries) => [...entries.filter(([n]) => n !== "signature.json"), ["future/data.bin", Buffer.from("new")]]);
    const unknownCheck = await checkPackage(unknown, { ...notesV, signer: undefined, sha256: await sha256Hex(unknown) }, "dev.plinth.examples.notes");
    assert.equal(unknownCheck.signature.status, "unsigned");
    const unknownPath = path.join(dir, "unknown.plnt");
    writeFileSync(unknownPath, unknown);
    execFileSync(exe, ["sign", unknownPath], { env, stdio: "ignore" });
    const unknownSigned = new Uint8Array(readFileSync(unknownPath));
    assert.ok(openZip(unknownSigned).has("future/data.bin"), "plinth sign keeps the unknown entry");
    const signedCheck = await checkPackage(unknownSigned, { ...notesV, sha256: await sha256Hex(unknownSigned) }, "dev.plinth.examples.notes");
    assert.equal(signedCheck.signature.status, "verified", "the signature covers the unknown entry");

    // The core digest from the core list.
    const cores = JSON.parse(readFileSync(path.join(reg, "cores", "index.json"), "utf8")).cores;
    const coreDir = readdirSync(path.join(reg, "cores")).find((d) => d !== "index.json");
    const coreBytes = new Uint8Array(readFileSync(path.join(reg, "cores", coreDir, "core.wasm")));
    await checkCore(coreBytes, cores[0]);
    await rejects(checkCore(coreBytes.slice(1), cores[0]), /does not match the registry digest/);

    console.log("run-hub-integrity.mjs: ok (digests, manifest, Ed25519 signature over every entry, changed manifests refused, newer profile, unknown entries, not-checked mode, core digest)");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
