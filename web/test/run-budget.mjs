// Node test: loads examples/budget/dist/budget.plnt and a core exported
// with `plinth core export`, through the same host code the browser uses.
// Checks `Row.trailing` (UI API 1.4, SPEC.md §6.3): the web renderer sets
// the `trailing` prop on transaction rows to the signed amount.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/budget
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";
import { ControlKind, Prop } from "../ui-api.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/budget/dist/budget.plnt")));
  const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
  const { appWasm } = await readPlnt(pltBytes);

  const app = new PlinthApp();
  const tree = buildTreeTracker(app);
  await app.load(coreBytes, appWasm, { log: () => {} });

  app.init([]);

  const rowIds = [...tree.kind.entries()].filter(([, k]) => k === ControlKind.row).map(([id]) => id);
  assert.ok(rowIds.length > 0, "expected at least one Row node");

  const trailingValues = rowIds.map((id) => tree.props.get(id)?.get(Prop.trailing)).filter((v) => v !== undefined);
  assert.ok(trailingValues.length > 0, "expected at least one Row with a trailing value");
  // The seed data (examples/budget/app/model.ts) has a +2500.00 salary row.
  assert.ok(trailingValues.includes("+2500.00"), `expected a trailing value "+2500.00", got: ${trailingValues.join(", ")}`);
  // Row.title is the category, not the amount, now that the amount moved
  // to `trailing`.
  const titleValues = rowIds.map((id) => tree.props.get(id)?.get(Prop.title));
  assert.ok(titleValues.includes("Salary"), `expected a title "Salary", got: ${titleValues.join(", ")}`);

  console.log("run-budget.mjs: ok");
}

function buildTreeTracker(app) {
  const kind = new Map();
  const props = new Map();
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "create") {
        kind.set(op.id, op.kind);
      } else if (op.op === "remove") {
        kind.delete(op.id);
        props.delete(op.id);
      } else if (op.op === "set-prop") {
        if (!props.has(op.id)) props.set(op.id, new Map());
        props.get(op.id).set(op.prop, op.value);
      }
    }
  };
  return { kind, props };
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
