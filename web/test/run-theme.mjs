// Test of the per-app theme on the web host (docs/ui.md, "Theming an app").
//
// Part 1 (Node, no browser): `web/theme.js` reads `[theme]` from a manifest,
// checks it (keys, hex, count, contrast in light and dark), merges it with
// the host palette and writes CSS for the app root only. The golden file
// `web/test/fixtures/theme-golden.json` is the same file that
// crates/plinth-package/tests/web_golden.rs checks, so the web host and the
// desktop host give the same colors for the same manifest. A failing override
// is ignored (a warning, the host theme stays), and nothing throws.
// Part 2 (Playwright, skipped when it is not installed): `examples/themed` in
// a real page: the CSS variables of the app root have the brand values in
// light and dark, high contrast and forced colors ignore them, the host
// banner keeps the host colors, and a failing override falls back.
//
// Build first:
//   cargo build -p plinth-cli
//   target/debug/plinth.exe core export target/core.wasm
//   target/debug/plinth.exe build examples/themed
// Part 2 needs the Playwright npm package (found with `require`, or in the
// folder of the environment variable PLAYWRIGHT_PATH) and a Chromium that
// Playwright can start.

import { createServer } from "node:http";
import { createRequire } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { readPlnt } from "../plinth-web.js";
import { THEME_ATTR, applyAppTheme, parseManifestTheme, resolveManifestTheme, themeCss, contrast, parseHex } from "../theme.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

// -- Part 1: the rules ---------------------------------------------------------

const golden = JSON.parse(readFileSync(path.join(root, "web/test/fixtures/theme-golden.json"), "utf-8"));
const manifestOf = (c) => `id = "dev.plinth.test"\nname = "T"\n${c.accent ? `accent = "${c.accent}"\n` : ""}\n${c.theme_toml}`;

for (const c of golden.ok) {
  const r = resolveManifestTheme(manifestOf(c));
  assert.equal(r.state, "ok", `${c.name}: ${r.reason}`);
  assert.deepEqual(r.light, c.light, `${c.name}: light`);
  assert.deepEqual(r.dark, c.dark, `${c.name}: dark`);
}
for (const c of golden.fail) {
  const r = resolveManifestTheme(manifestOf(c));
  assert.equal(r.state, "ignored", `${c.name} must be ignored`);
  for (const needle of c.error_contains) assert.ok(r.reason.includes(needle), `${c.name}: "${r.reason}" lacks "${needle}"`);
}

// No `[theme]`: nothing changes.
assert.deepEqual(resolveManifestTheme('id = "a.b"\nname = "A"\naccent = "teal"\n\n[[capabilities]]\nname = "store.kv"\nrationale = "x"\n'), { state: "none" });
assert.deepEqual(resolveManifestTheme(""), { state: "none" });

// The manifest that the Rust writer makes for `examples/themed` (sub-tables only, no `[theme]` line).
const planted = readFileSync(path.join(root, "examples/themed/plinth.toml"), "utf-8");
assert.ok(planted.includes("[theme]"));
const packagePath = path.join(root, "examples/themed/dist/themed.plnt");
let themedManifest = null;
if (existsSync(packagePath)) {
  themedManifest = (await readPlnt(new Uint8Array(readFileSync(packagePath)))).manifestText;
  const r = resolveManifestTheme(themedManifest);
  assert.equal(r.state, "ok", r.reason);
  assert.equal(r.light["--pl-bg"], "#fff8ee");
  assert.equal(r.dark["--pl-bg"], "#1b1410");
  assert.equal(r.light["--pl-accent"], "#b4440a");
  assert.equal(r.dark["--pl-accent"], "#ff9a4d");
  // The same colors as the first golden case (the same theme).
  assert.deepEqual(r.light, golden.ok[0].light);
  assert.deepEqual(r.dark, golden.ok[0].dark);
} else {
  console.log("run-theme.mjs: examples/themed/dist/themed.plnt is not built; the package check is skipped");
}

// Contrast math: the same numbers as the Rust rules.
assert.ok(Math.abs(contrast(0x000000, 0xffffff) - 21) < 0.001);
assert.ok(Math.abs(contrast(0x777777, 0xffffff) - 4.48) < 0.01);
assert.equal(parseHex("#AbCdEf"), 0xabcdef);
for (const bad of ["red", "#fff", "#12345", "#1234567", "#gg0000", "123456", " #123456", 5, null]) assert.equal(parseHex(bad), null, String(bad));

// Hostile and odd manifest text never throws and never gives a theme that fails the rules.
for (const text of [
  "[theme",
  "[theme]\n\u0000\n",
  "[theme]\n__proto__ = \"#112233\"\n",
  "[theme]\nconstructor = \"#112233\"\n",
  "[theme.__proto__]\nlight = \"#112233\"\ndark = \"#112233\"\n",
  "[theme.text]\nlight = \"#000000\"\n",
  "[theme.text]\nlight = \"#000000\"\ndark = \"#ffffff\"\nextra = \"#ffffff\"\n",
  "[theme.text]\n[theme.text]\n",
  "[theme]\ntext = \"#000000\"\ntext = \"#ffffff\"\n",
  "[theme]\ntext = { light = \"#000000\", dark = \"#ffffff\", extra = \"#ffffff\" }\n",
  "[theme]\ntext = [\"#000000\"]\n",
  `[theme]\n${"a".repeat(100000)} = "#000000"\n`,
  "[theme]\nbackground = \"#ffffff\"\" \n",
]) {
  const r = resolveManifestTheme(text);
  assert.notEqual(r.state, "ok", JSON.stringify(text).slice(0, 80));
}
// An empty `[theme]` is valid and changes nothing (as in Rust).
assert.equal(resolveManifestTheme("[theme]\n").state, "ok");
// An unreadable value is ignored as a whole.
assert.equal(resolveManifestTheme("[theme]\ntext = 5\n").state, "ignored");
// A key named like an object property is an unknown key, not a crash.
assert.ok(resolveManifestTheme("[theme]\n__proto__ = \"#112233\"\n").reason.includes("unknown key"));
// The theme of another table is not read.
assert.equal(resolveManifestTheme('[updates]\nfeed = "x"\n[theme.other]\n').state, "ignored");
assert.equal(parseManifestTheme('[updates]\ntheme = "x"\n').theme, null);

// The CSS: for the app root only, and not in high contrast or forced colors.
const css = themeCss("[data-pl-theme]", resolveManifestTheme(manifestOf(golden.ok[0])));
assert.ok(css.includes("(not (prefers-contrast: more))") && css.includes("(not (forced-colors: active))"), css);
assert.ok(css.includes("(prefers-color-scheme: light)"));
assert.ok(!/(^|\n)\s*(:root|html|body)\b/.test(css), "no rule for the page");
assert.equal(css.match(/\[data-pl-theme\]/g).length, 2);
assert.ok(css.includes("--pl-bg: #1b1410;") && css.includes("--pl-bg: #fff8ee;"));

// The host banners and the focus ring: `style.css` computes the host colors on the root.
const style = readFileSync(path.join(root, "web/style.css"), "utf-8");
assert.ok(/--pl-host-danger:\s*var\(--pl-danger\)/.test(style) && /background:\s*var\(--pl-host-danger\)/.test(style), "the banner uses the host colors");

// -- applyAppTheme with a small fake document ------------------------------------

function fakeDocument() {
  const head = { children: [], appendChild(el) { this.children.push(el); el.parent = this; } };
  const doc = {
    head,
    createElement: (tag) => ({ tag, id: "", textContent: "", remove() { head.children = head.children.filter((c) => c !== this); } }),
    getElementById: (id) => head.children.find((c) => c.id === id) ?? null,
  };
  const attrs = new Map();
  const container = {
    ownerDocument: doc,
    setAttribute: (k, v) => attrs.set(k, v),
    removeAttribute: (k) => attrs.delete(k),
    hasAttribute: (k) => attrs.has(k),
  };
  return { doc, container };
}

{
  const { doc, container } = fakeDocument();
  const warnings = [];
  const warn = (m) => warnings.push(m);
  const good = manifestOf(golden.ok[0]);
  assert.equal(applyAppTheme(container, good, warn).state, "ok");
  assert.ok(container.hasAttribute(THEME_ATTR));
  assert.equal(doc.head.children.length, 1);
  assert.ok(doc.head.children[0].textContent.includes("--pl-accent: #b4440a;"));
  // Applying again replaces the style (one style element).
  applyAppTheme(container, good, warn);
  assert.equal(doc.head.children.length, 1);
  // A failing override: the theme is removed, a warning is logged, the host theme stays.
  const bad = manifestOf(golden.fail[0]);
  assert.equal(applyAppTheme(container, bad, warn).state, "ignored");
  assert.ok(!container.hasAttribute(THEME_ATTR));
  assert.equal(doc.head.children.length, 0);
  assert.equal(warnings.length, 1);
  assert.ok(warnings[0].includes("the app theme is ignored") && warnings[0].includes("`text` on `background`"), warnings[0]);
  // No theme: no attribute, no style, no warning.
  assert.equal(applyAppTheme(container, 'id = "a.b"\n', warn).state, "none");
  assert.equal(warnings.length, 1);
  // Not a string, and a container with no document: never throws.
  applyAppTheme(container, undefined, warn);
  applyAppTheme({ ownerDocument: null, setAttribute() {}, removeAttribute() {} }, good, warn);
}

console.log("run-theme.mjs: part 1 (rules) ok");

// -- Part 2: a real page ---------------------------------------------------------

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  for (const where of [undefined, process.env.PLAYWRIGHT_PATH]) {
    try {
      return require(where ? path.resolve(where) : "playwright");
    } catch {
      // try the next place
    }
  }
  return null;
}

const MIME = { ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".plnt": "application/zip", ".json": "application/json" };

function startServer(dir) {
  return new Promise((resolve, reject) => {
    const server = createServer(async (req, res) => {
      try {
        const urlPath = decodeURIComponent(req.url.split("?")[0]);
        const filePath = path.join(dir, urlPath);
        if (!filePath.startsWith(dir)) {
          res.writeHead(403);
          res.end();
          return;
        }
        const data = await readFile(filePath);
        res.writeHead(200, { "Content-Type": MIME[path.extname(filePath)] ?? "application/octet-stream" });
        res.end(data);
      } catch (err) {
        res.writeHead(404);
        res.end(String(err?.message ?? err));
      }
    });
    server.listen(0, "127.0.0.1", () => resolve(server));
    server.on("error", reject);
  });
}

const playwright = loadPlaywright();
if (!playwright) {
  console.log("run-theme.mjs: part 2 skipped (the Playwright npm package is not installed; set PLAYWRIGHT_PATH)");
  process.exit(0);
}
if (!themedManifest || !existsSync(path.join(root, "target/core.wasm"))) {
  console.log("run-theme.mjs: part 2 skipped (build examples/themed and export target/core.wasm first)");
  process.exit(0);
}

const browser = await playwright[process.env.PLINTH_BROWSER ?? "chromium"].launch();
const server = await startServer(root);
try {
  const base = `http://127.0.0.1:${server.address().port}`;
  const page = await (await browser.newContext()).newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(`${base}/web/index.html?app=/examples/themed/dist/themed.plnt&core=/target/core.wasm`);
  await page.waitForSelector("#app[data-pl-theme] .pl-screen");

  /** The computed values of the tokens on `selector`. */
  const read = (selector, names = ["--pl-bg", "--pl-text", "--pl-accent", "--pl-accent-text", "--pl-selected"]) =>
    page.evaluate(
      ({ selector, names }) => {
        const el = document.querySelector(selector);
        const cs = getComputedStyle(el);
        return { vars: Object.fromEntries(names.map((n) => [n, cs.getPropertyValue(n).trim()])), background: cs.backgroundColor, color: cs.color, scheme: cs.colorScheme };
      },
      { selector, names },
    );

  // Dark (the default of style.css): the brand dark values on the app root.
  await page.emulateMedia({ colorScheme: "dark" });
  let app = await read("#app");
  assert.equal(app.vars["--pl-bg"], "#1b1410");
  assert.equal(app.vars["--pl-accent"], "#ff9a4d");
  assert.equal(app.background, "rgb(27, 20, 16)");
  assert.equal(app.color, "rgb(247, 234, 219)");
  // The page outside the app root keeps the host theme.
  const hostBg = (await read("html")).vars["--pl-bg"];
  assert.equal(hostBg, "#0b0c0f");
  // Descendants inherit the brand values.
  assert.equal((await read(".pl-screen")).vars["--pl-accent"], "#ff9a4d");
  // The host banner keeps the host colors, also inside the themed root.
  const banner = await read(".pl-host-banner", ["--pl-host-danger", "--pl-danger"]);
  assert.equal(banner.vars["--pl-host-danger"], "#e5484d");
  assert.equal(banner.vars["--pl-danger"], "#ff8a80");

  // Light.
  await page.emulateMedia({ colorScheme: "light" });
  app = await read("#app");
  assert.equal(app.vars["--pl-bg"], "#fff8ee");
  assert.equal(app.vars["--pl-accent"], "#b4440a");
  assert.equal(app.vars["--pl-selected"], "rgb(180 68 10 / 0.12)");
  assert.equal(app.background, "rgb(255, 248, 238)");
  assert.equal((await read("html")).vars["--pl-bg"], "#f5f6f8");

  // High contrast: the override is ignored; the host high-contrast tokens win.
  await page.emulateMedia({ colorScheme: "light", contrast: "more" });
  app = await read("#app");
  assert.equal(app.vars["--pl-bg"], "#ffffff");
  assert.equal(app.vars["--pl-text"], "#000000");
  assert.equal(app.vars["--pl-accent"], "#1f3fa8");
  await page.emulateMedia({ colorScheme: "dark", contrast: "more" });
  app = await read("#app");
  assert.equal(app.vars["--pl-bg"], "#000000");
  assert.equal(app.vars["--pl-accent"], "#a8b8ff");
  // Forced colors: the system colors win.
  await page.emulateMedia({ colorScheme: "light", contrast: "no-preference", forcedColors: "active" });
  app = await read("#app");
  assert.ok(!["#fff8ee", "#1b1410"].includes(app.vars["--pl-bg"]), `forced colors: ${app.vars["--pl-bg"]}`);
  assert.equal(app.vars["--pl-bg"].toLowerCase(), "canvas");
  await page.emulateMedia({ forcedColors: "none", contrast: "no-preference", colorScheme: "light" });
  assert.equal((await read("#app")).vars["--pl-bg"], "#fff8ee");

  // A failing override falls back to the host theme (a warning, no throw).
  const fallback = await page.evaluate(async () => {
    const { applyAppTheme } = await import("/web/theme.js");
    const div = document.createElement("div");
    document.body.appendChild(div);
    const warnings = [];
    const state = applyAppTheme(div, 'id = "a.b"\n[theme]\ntext = "#eeeeee"\n', (m) => warnings.push(m)).state;
    const cs = getComputedStyle(div);
    return { state, warnings, attr: div.hasAttribute("data-pl-theme"), bg: cs.getPropertyValue("--pl-bg").trim(), host: getComputedStyle(document.documentElement).getPropertyValue("--pl-bg").trim() };
  });
  assert.equal(fallback.state, "ignored");
  assert.equal(fallback.attr, false);
  assert.equal(fallback.bg, fallback.host);
  assert.equal(fallback.warnings.length, 1);
  assert.ok(fallback.warnings[0].includes("`text` on `background`"));

  assert.deepEqual(errors, []);
  console.log("run-theme.mjs: part 2 (page) ok");
} finally {
  await browser.close();
  server.close();
}
