// Node test: `readPlnt` exposes a `.plnt`'s `assets/` entries (SPEC.md
// §10.1, `<Image>`), by path without the `assets/` prefix, with their raw
// bytes. Does not touch the DOM (dom-renderer.js builds blob URLs from
// this map; that part needs a browser).
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe build examples/gallery

import { readFileSync } from "node:fs";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { readPlnt } from "../plinth-web.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

async function main() {
  const pltBytes = new Uint8Array(readFileSync(path.join(root, "examples/gallery/dist/gallery.plnt")));
  const { assets } = await readPlnt(pltBytes);

  const names = [...assets.keys()].sort();
  assert.deepEqual(names, ["badge.png", "banner.png", "mountain.png"], `unexpected asset names: ${names}`);
  for (const name of names) {
    const bytes = assets.get(name);
    assert.ok(bytes instanceof Uint8Array && bytes.length > 0, `${name} should have non-empty bytes`);
    // A PNG signature, so the bytes are not corrupted by zip inflate.
    assert.deepEqual([...bytes.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], `${name} is not a PNG`);
  }

  console.log("run-gallery.mjs: ok (%d assets: %s)", names.length, names.join(", "));
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
