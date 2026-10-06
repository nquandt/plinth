// Package checks of the web App Hub (docs/web-hub.md §4.3, docs/REGISTRY.md
// §5, docs/HUB.md §6.1): before the hub page runs a package, it checks the
// SHA-256 digest from the registry, the manifest, and the publisher
// signature (Ed25519) if the package has one. It uses WebCrypto
// (`crypto.subtle`), which browsers give only to a secure page (HTTPS or
// `localhost`) and which Node 22 has. No DOM code: `test/run-hub-integrity.mjs`
// runs this file directly.

import { openZip } from "./zip.js";

/** Lower-case hex of the SHA-256 digest of `bytes`. */
export async function sha256Hex(bytes) {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) throw new Error("this page cannot compute digests (WebCrypto needs HTTPS or localhost)");
  const digest = await subtle.digest("SHA-256", bytes);
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function tomlString(text, key) {
  const m = new RegExp(`^${key}\\s*=\\s*"((?:[^"\\\\]|\\\\.)*)"`, "m").exec(text);
  return m ? m[1] : null;
}

/** JSON with the object keys sorted and no white space (the `canonical_json` of `plinth-package`). */
export function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value && typeof value === "object") {
    const keys = Object.keys(value).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
    return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(value[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function base64ToBytes(text, url = false) {
  let s = String(text).trim();
  if (url) s = s.replace(/-/g, "+").replace(/_/g, "/");
  while (s.length % 4) s += "=";
  const bin = atob(s);
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

/** True if this browser can verify Ed25519 signatures with WebCrypto. */
export async function ed25519Supported() {
  try {
    await globalThis.crypto.subtle.importKey("raw", new Uint8Array(32).fill(1), { name: "Ed25519" }, false, ["verify"]);
    return true;
  } catch {
    return false;
  }
}

async function ed25519Verify(publicKey, signature, message) {
  const key = await globalThis.crypto.subtle.importKey("raw", publicKey, { name: "Ed25519" }, false, ["verify"]);
  return globalThis.crypto.subtle.verify({ name: "Ed25519" }, key, signature, message);
}

/**
 * Checks `signature.json` of a package (`plinth-package/src/signature.rs`):
 * the digest of each entry, the publisher, and the Ed25519 signature over
 * the canonical statement. Returns `{ status, key, publisher, note }`:
 * - `verified`: the signature is correct.
 * - `not-checked`: the entry digests are correct, but this browser cannot
 *   verify Ed25519 (WebCrypto has no Ed25519). The page says so and runs
 *   the app; the registry digest still protects the package.
 * Throws if the signature does not agree with the package.
 */
export async function checkSignature(zip, manifest, sigBytes, { canVerify } = {}) {
  let file;
  try {
    file = JSON.parse(new TextDecoder().decode(sigBytes));
  } catch {
    throw new Error("signature.json is not valid JSON");
  }
  if (file?.schema !== "plinth.signature/1") throw new Error(`signature.json has an unknown schema "${file?.schema}"`);
  if (file.publisher !== manifest.publisher) {
    throw new Error(`the manifest's publisher ("${manifest.publisher}") does not match the signature's ("${file.publisher}")`);
  }
  const expected = {};
  for (const name of zip.names()) {
    if (name === manifest.entry || name.startsWith("assets/")) expected[name] = await sha256Hex(await zip.read(name));
  }
  if (canonicalJson(expected) !== canonicalJson(file.digests ?? {})) {
    throw new Error("the package does not match its signature (a byte changed, or an entry was added or removed)");
  }
  if (typeof file.key !== "string" || !file.key.startsWith("ed25519:")) throw new Error(`"${file.key}" is not an ed25519 key id`);
  const publicKey = base64ToBytes(file.key.slice("ed25519:".length), true);
  if (publicKey.length !== 32) throw new Error(`"${file.key}" is not a 32-byte key`);
  const signature = base64ToBytes(file.signature ?? "");
  if (signature.length !== 64) throw new Error("the signature is not 64 bytes");
  const supported = canVerify ?? (await ed25519Supported());
  if (!supported) {
    return { status: "not-checked", key: file.key, publisher: file.publisher, note: "This browser cannot check Ed25519 signatures." };
  }
  const statement = { schema: file.schema, publisher: file.publisher, key: file.key, digests: file.digests };
  const ok = await ed25519Verify(publicKey, signature, new TextEncoder().encode(canonicalJson(statement)));
  if (!ok) throw new Error("the package signature does not verify");
  return { status: "verified", key: file.key, publisher: file.publisher, note: "" };
}

/**
 * Checks a downloaded package against its registry entry (an app-document
 * version, docs/REGISTRY.md §4) before the page runs it:
 * 1. the SHA-256 of the file is `version.sha256`;
 * 2. the manifest's `id` is `appId` and its `version` is `version.version`;
 *    the manifest's `digest` is the digest of the app module;
 * 3. the signature, if the package has one, and the signer agrees with the
 *    registry (`version.signer`).
 *
 * Returns `{ manifestText, manifest, signature }`; throws with a plain
 * reason on a mismatch. `opts.canVerify` overrides the Ed25519 detection
 * (tests).
 */
export async function checkPackage(bytes, version, appId, opts = {}) {
  const want = String(version?.sha256 ?? "").toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(want)) throw new Error("the registry gives no SHA-256 digest for this version");
  const got = await sha256Hex(bytes);
  if (got !== want) throw new Error(`the package does not match the registry digest (expected ${want.slice(0, 12)}…, got ${got.slice(0, 12)}…)`);

  const zip = openZip(bytes);
  if (!zip.has("manifest.toml")) throw new Error("the package has no manifest.toml");
  const manifestText = new TextDecoder().decode(await zip.read("manifest.toml"));
  const manifest = {
    id: tomlString(manifestText, "id") ?? "",
    version: tomlString(manifestText, "version") ?? "",
    publisher: tomlString(manifestText, "publisher") ?? "",
    entry: tomlString(manifestText, "entry") ?? "app.wasm",
    digest: tomlString(manifestText, "digest") ?? "",
  };
  if (manifest.id.toLowerCase() !== String(appId).toLowerCase()) throw new Error(`the package is the app "${manifest.id}", not "${appId}"`);
  if (version.version && manifest.version !== version.version) {
    throw new Error(`the package is version ${manifest.version}, not ${version.version}`);
  }
  if (!zip.has(manifest.entry)) throw new Error(`the manifest entry ${manifest.entry} is not in the package`);
  if (manifest.digest && (await sha256Hex(await zip.read(manifest.entry))) !== manifest.digest.toLowerCase()) {
    throw new Error(`the digest of ${manifest.entry} does not match the manifest`);
  }

  let signature = { status: "unsigned", key: null, publisher: null, note: "" };
  if (zip.has("signature.json")) {
    signature = await checkSignature(zip, manifest, await zip.read("signature.json"), opts);
  }
  if (version.signer && signature.key !== version.signer) {
    throw new Error(
      signature.key ? `the package is signed by ${signature.key}, but the registry says ${version.signer}` : "the registry says that this version is signed, but the package has no signature",
    );
  }
  return { manifestText, manifest, signature };
}

/** Checks a core file against its entry in the registry core list (`{ version, sha256 }`). */
export async function checkCore(bytes, core) {
  if (!core?.sha256) return;
  const got = await sha256Hex(bytes);
  if (got !== String(core.sha256).toLowerCase()) throw new Error(`the runtime core ${core.version} does not match the registry digest`);
}
