// Plain node test for the JSON -> diagnostic mapping (no VS Code needed).
// Run after `npm run build` (needs out/mapDiagnostics.js): `npm test`.
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { parsePlinthJson, mapDiagnostics, groupByFile, statusText } = require("../out/mapDiagnostics.js");

const SAMPLE = JSON.stringify([
  {
    file: "app/main.tsx",
    line: 3,
    column: 5,
    start: 20,
    end: 26,
    severity: "error",
    code: "PL1007",
    message: "missing capability 'store.kv'",
    help: "declare it in plinth.toml",
  },
  {
    file: "app/main.tsx",
    line: 7,
    column: 1,
    start: 40,
    end: 41,
    severity: "warning",
    code: "PL2020",
    message: "signal read outside a reactive position",
    help: null,
  },
]);

test("parsePlinthJson parses the array", () => {
  const parsed = parsePlinthJson(SAMPLE);
  assert.equal(parsed.length, 2);
  assert.equal(parsed[0].code, "PL1007");
});

test("parsePlinthJson rejects non-array JSON", () => {
  assert.throws(() => parsePlinthJson("{}"));
});

test("mapDiagnostics converts to 0-based ranges", () => {
  const [first] = mapDiagnostics(parsePlinthJson(SAMPLE));
  assert.deepEqual(first.range.start, { line: 2, character: 4 });
  assert.deepEqual(first.range.end, { line: 2, character: 4 + (26 - 20) });
  assert.equal(first.severity, "error");
  assert.equal(first.help, "declare it in plinth.toml");
});

test("mapDiagnostics clamps zero-width spans to width 1", () => {
  const [, second] = mapDiagnostics(parsePlinthJson(SAMPLE));
  assert.equal(second.range.end.character - second.range.start.character, 1);
  assert.equal(second.help, null);
});

test("groupByFile groups diagnostics per file", () => {
  const groups = groupByFile(mapDiagnostics(parsePlinthJson(SAMPLE)));
  assert.equal(groups.size, 1);
  assert.equal(groups.get("app/main.tsx").length, 2);
});

test("statusText reports ok or an error count", () => {
  assert.equal(statusText([]), "Plinth: ok");
  assert.equal(statusText(parsePlinthJson(SAMPLE)), "Plinth: 1 error");
  const twoErrors = JSON.parse(SAMPLE);
  twoErrors[1].severity = "error";
  assert.equal(statusText(twoErrors), "Plinth: 2 errors");
});
