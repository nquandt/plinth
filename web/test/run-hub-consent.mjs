// Node test (docs/web-hub.md §4.2, docs/HUB.md §7.2, §7.3): the consent
// logic of the web App Hub (`hub-consent.js`), with no DOM. Risk-level
// defaults, decisions for one session (high risk), denied capabilities,
// re-consent when an update adds a capability, and the Manage page changes.

import assert from "node:assert/strict";
import {
  riskOf,
  riskDefaults,
  manifestCapabilities,
  consentRows,
  needsConsent,
  effectiveGrants,
  recordDecisions,
  setGrant,
  decisionWords,
} from "../hub-consent.js";

const MANIFEST_1 = `id = "com.example.tool"
name = "Tool"
version = "1.0.0"
publisher = "me"

[[capabilities]]
name = "store.kv"
rationale = "Keep your \\"settings\\"."

[[capabilities]]
name = "clipboard.read"
rationale = "Paste text."
`;

const MANIFEST_2 = `${MANIFEST_1}
[[capabilities]]
name = "net:*"
rationale = "Load any page."

[[capabilities]]
name = "net:api.example.com"
`;

const labels = [{ name: "store.kv", risk: "low", description: "save data on this device", rationale: "from the registry" }];

// Risk levels (the same as crates/plinth-link/src/capabilities.rs).
assert.equal(riskOf("store.kv"), "low");
assert.equal(riskOf("clipboard.write"), "low");
assert.equal(riskOf("clipboard.read"), "medium");
assert.equal(riskOf("net.local"), "medium");
assert.equal(riskOf("net:api.example.com"), "medium");
assert.equal(riskOf("net:*"), "high");
assert.equal(riskOf("hub.manage"), "high");
assert.equal(riskOf("camera"), "high", "an unknown capability is high risk");
assert.deepEqual(riskDefaults("none"), { ask: false, allow: true, session: false });
assert.deepEqual(riskDefaults("low"), { ask: true, allow: true, session: false });
assert.deepEqual(riskDefaults("medium"), { ask: true, allow: false, session: false });
assert.deepEqual(riskDefaults("high"), { ask: true, allow: false, session: true });

// The manifest gives the names and the reasons.
const caps1 = manifestCapabilities(MANIFEST_1);
assert.deepEqual(caps1, [
  { name: "store.kv", rationale: 'Keep your "settings".' },
  { name: "clipboard.read", rationale: "Paste text." },
]);
assert.deepEqual(manifestCapabilities('id = "x"\n'), []);

// First run: nothing is decided, the consent view comes first.
const app = { id: "com.example.tool", name: "Tool", version: "1.0.0" };
let rows = consentRows(caps1, labels, null, null);
assert.equal(needsConsent(rows), true);
assert.deepEqual(
  rows.map((r) => [r.name, r.risk, r.defaultAllow, r.decided, r.isNew]),
  [
    ["store.kv", "low", true, false, false],
    ["clipboard.read", "medium", false, false, false],
  ],
);
assert.equal(rows[0].description, "save data on this device", "words from the registry label");
assert.equal(rows[0].rationale, 'Keep your "settings".', "the reason from the manifest comes first");
assert.deepEqual(effectiveGrants(rows), { granted: [], refused: ["store.kv", "clipboard.read"] }, "nothing is granted silently");

// The user keeps the defaults: low allowed, medium denied.
let { record, sessionRecord } = recordDecisions(null, null, app, rows, { "store.kv": true, "clipboard.read": false }, "T1");
assert.equal(record.version, "1.0.0");
assert.deepEqual(record.capabilities, ["store.kv", "clipboard.read"]);
assert.deepEqual(record.grants["store.kv"], { allowed: true, at: "T1", byDefault: true });
assert.deepEqual(record.grants["clipboard.read"], { allowed: false, at: "T1", byDefault: true });
rows = consentRows(caps1, labels, record, sessionRecord);
assert.equal(needsConsent(rows), false, "the second run asks nothing");
assert.deepEqual(effectiveGrants(rows), { granted: ["store.kv"], refused: ["clipboard.read"] });
assert.equal(decisionWords(rows[0]), "Allowed (default)");
assert.equal(decisionWords(rows[1]), "Denied (default)");

// An update adds two capabilities: only they are new and undecided.
const caps2 = manifestCapabilities(MANIFEST_2);
rows = consentRows(caps2, labels, record, sessionRecord);
assert.equal(needsConsent(rows), true, "re-consent on an update with new capabilities");
assert.deepEqual(
  rows.filter((r) => r.isNew).map((r) => r.name),
  ["net:*", "net:api.example.com"],
);
assert.equal(rows.find((r) => r.name === "net:*").session, true);
({ record, sessionRecord } = recordDecisions(record, sessionRecord, { ...app, version: "1.1.0" }, rows, { "net:*": true, "net:api.example.com": true }, "T2"));
assert.equal(record.grants["store.kv"].at, "T1", "older decisions stay");
assert.equal(record.grants["net:*"], undefined, "a high-risk decision is not kept after the session");
assert.equal(sessionRecord.grants["net:*"].allowed, true);
assert.equal(record.grants["net:api.example.com"].allowed, true);
assert.equal(record.grants["net:api.example.com"].byDefault, false, "Allow is not the default for medium risk");
rows = consentRows(caps2, labels, record, sessionRecord);
assert.equal(needsConsent(rows), false);
assert.equal(decisionWords(rows.find((r) => r.name === "net:*")), "Allowed for this session");

// A new session: the high-risk capability is asked again, the others are not.
rows = consentRows(caps2, labels, record, null);
assert.deepEqual(
  rows.filter((r) => !r.decided).map((r) => r.name),
  ["net:*"],
);
assert.equal(rows.find((r) => r.name === "net:*").isNew, false, "asked again, but not new");

// The Manage page: change one grant.
({ record, sessionRecord } = setGrant(record, sessionRecord, "clipboard.read", true, "T3"));
assert.deepEqual(record.grants["clipboard.read"], { allowed: true, at: "T3", byDefault: false });
({ record, sessionRecord } = setGrant(record, sessionRecord, "store.kv", false, "T4"));
rows = consentRows(caps2, labels, record, sessionRecord);
assert.deepEqual(effectiveGrants(rows).refused, ["store.kv"]);
assert.equal(decisionWords(rows[0]), "Denied");
({ record, sessionRecord } = setGrant(record, sessionRecord, "net:*", false, "T5"));
assert.equal(sessionRecord.grants["net:*"].allowed, false);
assert.equal(record.grants["net:*"], undefined);

// The original objects do not change (the page saves the new ones).
const frozen = { schema: "plinth.web-grants/1", id: "x", name: "X", version: "1", capabilities: ["store.kv"], grants: {} };
const copy = JSON.stringify(frozen);
recordDecisions(frozen, null, { id: "x", name: "X", version: "2" }, consentRows([{ name: "store.kv", rationale: "" }], [], frozen, null), {});
setGrant(frozen, null, "store.kv", true);
assert.equal(JSON.stringify(frozen), copy);

console.log("run-hub-consent.mjs: ok (risk defaults, session decisions, re-consent on update, Manage changes)");
