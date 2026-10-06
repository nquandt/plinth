// Node test (SPEC.md §8.4, §8.5, §11, §18.3): `plinth:net`'s `fetch`
// against a local HTTP server (Node's own `http` module), so this test
// needs no internet access. Exercises the web host's capability check
// (`net.local` for a loopback host) and the GET/POST/denied paths.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build web/test/fixtures/net-fixture
//   target/debug/plinth.exe core export target/core.wasm

import { readFileSync } from "node:fs";
import http from "node:http";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { PlinthApp, readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const PORT = 18797; // must match web/test/fixtures/net-fixture/app/main.tsx

function startServer() {
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c));
    req.on("end", () => {
      if (req.method === "GET" && req.url === "/hello") {
        res.writeHead(200).end("hello world");
      } else if (req.method === "POST" && req.url === "/echo") {
        res.writeHead(200).end(`echo:${body}`);
      } else {
        res.writeHead(404).end("not found");
      }
    });
  });
  return new Promise((resolve) => server.listen(PORT, "127.0.0.1", () => resolve(server)));
}

/** Waits until `predicate()` is true or `ms` elapses (fetches complete
 * asynchronously: real sockets, even to localhost). */
async function waitFor(predicate, ms = 5000) {
  const start = Date.now();
  while (!predicate()) {
    if (Date.now() - start > ms) throw new Error("timed out waiting for the condition");
    await new Promise((r) => setTimeout(r, 5));
  }
}

async function main() {
  const server = await startServer();
  try {
    const pltBytes = new Uint8Array(readFileSync(path.join(root, "web/test/fixtures/net-fixture/dist/net-fixture.plnt")));
    const coreBytes = new Uint8Array(readFileSync(path.join(root, "target/core.wasm")));
    const { appWasm, manifestText } = await readPlnt(pltBytes);
    assert.ok(manifestText.includes('name = "net.local"'), "expected the fixture manifest to declare net.local");

    const app = new PlinthApp();
    const tree = buildTreeTracker(app);
    await app.load(coreBytes, appWasm, { log: () => {}, manifestText });
    app.init([]);

    const getHandler = findHandler(tree, 1 /* Event.press */, "get");
    const postHandler = findHandler(tree, 1, "post");
    const deniedHandler = findHandler(tree, 1, "denied");
    const awaitHandler = findHandler(tree, 1, "await");
    assert.ok(getHandler !== undefined && postHandler !== undefined && deniedHandler !== undefined, "missing button handlers");

    app.onEvent({ kind: "ui", handler: getHandler, event: 1, value: null });
    await waitFor(() => tree.text.get(tree.resultId) === "true:200:hello world");

    app.onEvent({ kind: "ui", handler: postHandler, event: 1, value: null });
    await waitFor(() => tree.text.get(tree.resultId) === "true:echo:hi");

    app.onEvent({ kind: "ui", handler: awaitHandler, event: 1, value: null });
    await waitFor(() => tree.text.get(tree.resultId) === "awaited:echo:hello world");

    app.onEvent({ kind: "ui", handler: deniedHandler, event: 1, value: null });
    await waitFor(() => tree.text.get(tree.resultId) === "false:denied:undeclared");

    console.log("run-net.mjs: ok (GET, POST, two awaited fetches and a denied undeclared host all completed)");
  } finally {
    server.close();
  }
}

/** Finds a node's handler id for `eventCode` whose last known label matches `label`. */
function findHandler(tree, eventCode, label) {
  for (const [id, byEvent] of tree.listen) {
    if (byEvent.has(eventCode) && tree.label.get(id) === label) {
      return byEvent.get(eventCode);
    }
  }
  return undefined;
}

/** A tiny tree tracker: text, labels and listeners, like run-notes.mjs's. */
function buildTreeTracker(app) {
  const text = new Map();
  const label = new Map();
  const listen = new Map();
  const tracker = { text, label, listen, resultId: undefined };
  app.onCommit = (ops) => {
    for (const op of ops) {
      if (op.op === "text") {
        text.set(op.id, String(op.value));
        if (tracker.resultId === undefined) tracker.resultId = op.id;
      } else if (op.op === "listen") {
        if (!listen.has(op.id)) listen.set(op.id, new Map());
        listen.get(op.id).set(op.event, op.handler);
      } else if (op.op === "set-prop" && typeof op.value === "string" && op.prop === 3 /* Prop.label */) {
        label.set(op.id, op.value);
      }
    }
  };
  return tracker;
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
