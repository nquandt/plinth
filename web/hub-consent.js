// Consent in the web App Hub (docs/web-hub.md §4, docs/HUB.md §7.2, §7.3):
// which capabilities of an app need a question before it runs, the risk
// level defaults, and the grant records that the hub page keeps for each
// app. No DOM and no storage code, so `test/run-hub-consent.mjs` runs it
// directly; `hub.js` renders it and `hub-storage.js` stores the records.
//
// The capability names come from the package manifest (after the digest
// check), not from the registry. The risk level comes from the table below,
// not from the registry: a client must not trust the registry labels for
// consent decisions (docs/REGISTRY.md §3). The registry labels give only
// the words (the description).

/**
 * The risk level of each capability, as in the shared capability map
 * (`crates/plinth-link/src/capabilities.rs`). `test/run-hub-logic.mjs`
 * checks that it agrees with the labels that `plinth registry build`
 * writes. An unknown name is "high".
 */
const RISKS = {
  "store.kv": "low",
  "clipboard.write": "low",
  "clipboard.read": "medium",
  "net.local": "medium",
  "net:*": "high",
  "hub.manage": "high",
};

export function riskOf(name) {
  if (Object.hasOwn(RISKS, name)) return RISKS[name];
  if (name.startsWith("net:")) return "medium"; // one named host
  return "high";
}

/**
 * The defaults of docs/HUB.md §7.2. `ask` is false for "none": the
 * capability is allowed with no question. `allow` is the choice that the
 * consent view selects first. `session` is true when a decision lasts only
 * for the browser session (high risk: "asked each time, or for a
 * session"; browse mode, docs/HUB.md §7.3 step 4).
 */
export function riskDefaults(risk) {
  switch (risk) {
    case "none":
      return { ask: false, allow: true, session: false };
    case "low":
      return { ask: true, allow: true, session: false };
    case "medium":
      return { ask: true, allow: false, session: false };
    default:
      return { ask: true, allow: false, session: true };
  }
}

/** Reads the `[[capabilities]]` tables of a `.plnt` manifest: `[{ name, rationale }]`. */
export function manifestCapabilities(manifestText) {
  const out = [];
  const blocks = String(manifestText).split(/^\s*\[\[capabilities\]\]\s*$/m).slice(1);
  for (const block of blocks) {
    const body = block.split(/^\s*\[/m)[0];
    const name = /^\s*name\s*=\s*"((?:[^"\\]|\\.)*)"/m.exec(body)?.[1];
    if (!name) continue;
    const rationale = /^\s*rationale\s*=\s*"((?:[^"\\]|\\.)*)"/m.exec(body)?.[1] ?? "";
    if (!out.some((c) => c.name === name)) out.push({ name, rationale: rationale.replace(/\\(.)/g, "$1") });
  }
  return out;
}

/** An empty grant record for an app. */
export function emptyRecord(id, name = id) {
  return { schema: "plinth.web-grants/1", id, name, version: null, capabilities: [], grants: {} };
}

/**
 * One row for each capability of the app, for the consent view and the
 * Manage page:
 * `{ name, risk, description, rationale, decided, allowed, byDefault, isNew, session, defaultAllow }`.
 *
 * `capabilities` is `manifestCapabilities(...)`; `labels` are the registry
 * labels (for the words only); `record` is the stored grant record (or
 * null); `sessionRecord` holds the decisions that last for this session.
 */
export function consentRows(capabilities, labels, record, sessionRecord) {
  const known = new Set(record?.capabilities ?? []);
  const hadVersion = Boolean(record?.version);
  return capabilities.map(({ name, rationale }) => {
    const risk = riskOf(name);
    const defaults = riskDefaults(risk);
    const label = (labels ?? []).find((l) => l.name === name);
    const stored = defaults.session ? sessionRecord?.grants?.[name] : record?.grants?.[name];
    const auto = !defaults.ask;
    return {
      name,
      risk,
      description: label?.description ?? "",
      rationale: rationale || label?.rationale || "",
      decided: auto || Boolean(stored),
      allowed: auto ? true : stored ? Boolean(stored.allowed) : null,
      byDefault: auto || Boolean(stored?.byDefault),
      isNew: hadVersion && !known.has(name) && !stored,
      session: defaults.session,
      defaultAllow: defaults.allow,
    };
  });
}

/** True if at least one capability has no decision yet: the consent view comes first. */
export function needsConsent(rows) {
  return rows.some((r) => !r.decided);
}

/**
 * The capabilities that the app may use and the ones that it may not. A
 * capability with no decision is refused (it is never granted silently).
 */
export function effectiveGrants(rows) {
  const granted = [];
  const refused = [];
  for (const r of rows) (r.decided && r.allowed ? granted : refused).push(r.name);
  return { granted, refused };
}

/**
 * Records the user's choices from the consent view. `decisions` maps a
 * capability name to true (Allow) or false (Deny); a row that the user
 * did not change keeps its default, marked `byDefault`. Returns new
 * `{ record, sessionRecord }` objects (the inputs do not change).
 */
export function recordDecisions(record, sessionRecord, app, rows, decisions, now = new Date().toISOString()) {
  const next = { ...(record ?? emptyRecord(app.id, app.name)), grants: { ...(record?.grants ?? {}) } };
  const session = { grants: { ...(sessionRecord?.grants ?? {}) } };
  for (const r of rows) {
    if (r.decided && !(r.name in decisions)) continue;
    if (!riskDefaults(r.risk).ask) continue;
    const chosen = r.name in decisions ? Boolean(decisions[r.name]) : r.defaultAllow;
    const entry = { allowed: chosen, at: now, byDefault: !(r.name in decisions) || chosen === r.defaultAllow };
    if (r.session) session.grants[r.name] = entry;
    else next.grants[r.name] = entry;
  }
  next.name = app.name ?? next.name;
  next.version = app.version ?? next.version;
  next.capabilities = rows.map((r) => r.name);
  return { record: next, sessionRecord: session };
}

/** Changes one grant (the Manage page). Returns new `{ record, sessionRecord }`. */
export function setGrant(record, sessionRecord, name, allowed, now = new Date().toISOString()) {
  const entry = { allowed: Boolean(allowed), at: now, byDefault: false };
  if (riskDefaults(riskOf(name)).session) {
    return { record, sessionRecord: { grants: { ...(sessionRecord?.grants ?? {}), [name]: entry } } };
  }
  return { record: { ...record, grants: { ...record.grants, [name]: entry } }, sessionRecord };
}

/** The words for a decision, for the Manage page. */
export function decisionWords(row) {
  if (!row.decided) return row.session ? "Not decided in this session" : "Not decided";
  const what = row.allowed ? "Allowed" : "Denied";
  if (!riskDefaults(row.risk).ask) return `${what} (no question for this risk level)`;
  if (row.session) return `${what} for this session`;
  return row.byDefault ? `${what} (default)` : what;
}
