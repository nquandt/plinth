// Node test (SPEC.md §11): `isPrivateNetHost`, the web host's `net.local`
// check. The cases match `private_host_forms_are_recognized` in
// `crates/plinth-runner-wasmtime/src/policy.rs`, so both hosts agree. Each
// host also goes through `new URL()`, as the real check does, so the
// normalized forms (`2130706433`, `[::ffff:7f00:1]`) are covered too.

import assert from "node:assert/strict";
import { isPrivateNetHost } from "../plinth-web.js";

const privateHosts = [
  "localhost", "LOCALHOST", "localhost.", "app.localhost", "printer.local", "127.0.0.1", "127.255.0.1",
  "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "100.64.0.1", "0.0.0.0",
  "0.1.2.3", "224.0.0.1", "255.255.255.255", "198.18.0.1", "192.0.0.8", "[::1]", "::1", "[::]", "[fd00::1]",
  "[fe80::1]", "[fec0::1]", "[::ffff:127.0.0.1]", "[::ffff:10.0.0.1]", "[::127.0.0.1]", "[64:ff9b::a9fe:a9fe]",
  "[2002:c0a8:0101::1]", "[ff02::1]", "127.0.0.1:8080", "[::1]:8080",
  // Forms that only `new URL()` turns into an IPv4 literal.
  "2130706433", "0x7f000001", "0177.0.0.1", "127.1",
];
const publicHosts = [
  "example.com", "8.8.8.8", "1.1.1.1", "172.32.0.1", "100.128.0.1", "[2606:4700::1111]",
  "[::ffff:8.8.8.8]", "[2002:0808:0808::1]", "localhost.example.com", "example.com:443",
];

// A bare IPv6 literal (`::1`) is not valid in a URL; such a host is tested as text only.
const viaUrl = (h) => (h.startsWith("[") || !h.includes("::") ? new URL(`http://${h}/`).hostname : h);
for (const h of privateHosts) {
  assert.ok(isPrivateNetHost(viaUrl(h)), `${h} (${viaUrl(h)}) should be private`);
  if (!/^(2130706433|0x7f000001|0177\.0\.0\.1|127\.1)$/.test(h)) assert.ok(isPrivateNetHost(h), `${h} should be private`);
}
for (const h of publicHosts) {
  assert.ok(!isPrivateNetHost(viaUrl(h)), `${h} (${viaUrl(h)}) should be public`);
  assert.ok(!isPrivateNetHost(h), `${h} should be public`);
}
console.log(`ok: ${privateHosts.length} private and ${publicHosts.length} public hosts`);
