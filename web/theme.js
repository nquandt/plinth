// Per-app theme token values on the web host (docs/ui.md, "Theming an app").
//
// An app can set the VALUES of the host color tokens in the `[theme]` table
// of its manifest. This module reads the table from `manifest.toml`, checks
// it again (a package is never trusted) and turns it into CSS custom
// properties for the app's root element only.
//
// The rules are the rules of crates/plinth-package/src/theme.rs: the same
// keys, the same hex-only values, the same host palette and the same
// contrast guard. `web/test/fixtures/theme-golden.json` is checked by a test
// on each side, so both hosts give the same colors for the same manifest.
// Keep the two files in sync.
//
// High contrast (`prefers-contrast: more`) and forced colors ignore the
// override: the CSS that this module writes is inside a media query that
// does not match then, so the host tokens of style.css win.

/** The color tokens that an app can override. */
export const THEME_KEYS = ["background", "surface", "surface-alt", "border", "hover", "selected", "text", "text-muted", "accent", "on-accent", "danger", "success"];
/** The most keys in a `[theme]` table. */
export const THEME_MAX_KEYS = 12;
const DEFAULT_ACCENT = "teal";

/** The named accents: [light, dark]. */
const ACCENTS = {
  teal: [0x0b8a7e, 0x2ec4b6],
  blue: [0x1a63d6, 0x5b9cff],
  indigo: [0x4f46c8, 0x8c86ff],
  purple: [0x8a3ec9, 0xc08cff],
  pink: [0xc8327a, 0xff7ab8],
  red: [0xc93a2f, 0xff7a6e],
  orange: [0xc25a00, 0xff9f45],
  green: [0x2e8540, 0x5fd27a],
};

/** The opacity of the `selected` tint that the host derives from the accent. */
const SELECTED_ALPHA = { light: 0.12, dark: 0.16 };

/** The host palette for an appearance and a named accent (the desktop palette). */
export function defaultPalette(dark, accent) {
  const [lightAccent, darkAccent] = ACCENTS[accent] ?? ACCENTS[DEFAULT_ACCENT];
  return dark
    ? {
        background: 0x161618,
        surface: 0x232326,
        "surface-alt": 0x2e2e32,
        border: 0x37373c,
        hover: 0x34343a,
        selected: null,
        text: 0xf2f2f5,
        "text-muted": 0x9a9aa2,
        accent: darkAccent,
        "on-accent": 0x0b0b0c,
        danger: 0xff6b5e,
        success: 0x5fd27a,
      }
    : {
        background: 0xf4f4f6,
        surface: 0xffffff,
        "surface-alt": 0xececf0,
        border: 0xdedee4,
        hover: 0xe4e4ea,
        selected: null,
        text: 0x1b1b1f,
        "text-muted": 0x6b6b74,
        accent: lightAccent,
        "on-accent": 0xffffff,
        danger: 0xc93a2f,
        success: 0x2e8540,
      };
}

/** Parses exactly `#rrggbb`; `null` for anything else. */
export function parseHex(s) {
  return typeof s === "string" && /^#[0-9a-fA-F]{6}$/.test(s) ? parseInt(s.slice(1), 16) : null;
}

/** The WCAG 2 relative luminance of `0xRRGGBB`. */
export function luminance(rgb) {
  const lin = (v) => {
    v /= 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin((rgb >> 16) & 0xff) + 0.7152 * lin((rgb >> 8) & 0xff) + 0.0722 * lin(rgb & 0xff);
}

/** The WCAG 2 contrast ratio of two colors (1 to 21). */
export function contrast(a, b) {
  const [la, lb] = [luminance(a), luminance(b)];
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** The pairs of the contrast guard: [text token, background token, least ratio]. */
const PAIRS = [
  ["text", "background", 4.5],
  ["text", "surface", 4.5],
  ["on-accent", "accent", 4.5],
  ["text-muted", "background", 3],
  ["danger", "background", 3],
  ["success", "background", 3],
];

// -- Reading `[theme]` from manifest.toml ---------------------------------------

/** A TOML basic string like `"#112233"`; `null` for any other value. */
function tomlString(text) {
  const m = text.trim().match(/^"([^"\\]*)"$/);
  return m ? m[1] : null;
}

/**
 * Reads the `accent` and the `[theme]` table from the text of `manifest.toml`.
 * Returns `{ accent, theme, error }`: `theme` is `null` when the manifest has
 * no `[theme]`, else an object key -> a hex string or `{ light, dark }`
 * (values are not checked here). `error` is a text when a line is not a form
 * that this reader knows (then the theme is ignored). It reads the forms that the Rust writer makes (`[theme]` and
 * `[theme.key]` tables) and inline tables.
 */
export function parseManifestTheme(manifestText) {
  const lines = String(manifestText).replace(/\r\n/g, "\n").split("\n");
  let accent = null;
  let section = null; // null | { key: null | string }: inside `[theme]` or `[theme.key]`
  let seen = false;
  const theme = Object.create(null);
  let sawHeader = false;
  for (const raw of lines) {
    const line = raw.trim();
    if (line === "" || line.startsWith("#")) continue;
    const header = line.match(/^\[\s*([^\]]*?)\s*\]$/);
    if (header && !line.startsWith("[[")) {
      sawHeader = true;
      const name = header[1];
      if (name === "theme") {
        section = { key: null };
        seen = true;
      } else if (name.startsWith("theme.")) {
        const key = name.slice(6);
        if (!/^[A-Za-z0-9_-]+$/.test(key) || Object.hasOwn(theme, key)) return { accent, theme: null, error: "a `[theme]` table that this reader does not know" };
        section = { key };
        theme[key] = {};
        seen = true;
      } else {
        section = null;
      }
      continue;
    }
    if (line.startsWith("[[")) {
      sawHeader = true;
      section = null;
      continue;
    }
    const kv = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.*)$/);
    if (!kv) {
      if (section) return { accent, theme: null, error: "a `[theme]` line that this reader does not know" };
      continue;
    }
    const [, k, v] = kv;
    if (!sawHeader && k === "accent") {
      accent = tomlString(v);
      continue;
    }
    if (!section) continue;
    if (section.key !== null) {
      // Inside `[theme.key]`: `light` and `dark`.
      const s = tomlString(v);
      if (s === null) return { accent, theme: null, error: `\`[theme] ${section.key}\` must be a hex string or a table with \`light\` and \`dark\` hex strings` };
      theme[section.key][k] = s;
    } else {
      const s = tomlString(v);
      if (s !== null) {
        theme[k] = s;
        continue;
      }
      const inline = v.trim().match(/^\{\s*(.*?)\s*\}$/);
      const entry = {};
      if (inline) {
        for (const part of inline[1].split(",")) {
          const pair = part.trim().match(/^([A-Za-z0-9_-]+)\s*=\s*(.*)$/);
          const ps = pair ? tomlString(pair[2]) : null;
          if (ps === null) return { accent, theme: null, error: `\`[theme] ${k}\` must be a hex string or a table with \`light\` and \`dark\` hex strings` };
          entry[pair[1]] = ps;
        }
      } else {
        return { accent, theme: null, error: `\`[theme] ${k}\` must be a hex string or a table with \`light\` and \`dark\` hex strings` };
      }
      theme[k] = entry;
    }
  }
  return { accent, theme: seen ? theme : null, error: null };
}

// -- Checks and merge -------------------------------------------------------------

/** The text of an override color for an appearance; `null` when it has none. */
function colorText(color, dark) {
  if (typeof color === "string") return color;
  if (color && typeof color === "object") {
    const keys = Object.keys(color);
    if (keys.length === 2 && typeof color.light === "string" && typeof color.dark === "string") return dark ? color.dark : color.light;
  }
  return null;
}

/**
 * Checks an override and merges it with the host palette, for both
 * appearances, as `ThemeOverride::resolve` does. Returns `{ ok: true,
 * light, dark }` (the palettes) or `{ ok: false, reason }`.
 *
 * A contrast pair is checked when the override sets one of its tokens.
 */
export function resolveOverride(theme, namedAccent) {
  if (theme === null || typeof theme !== "object" || Array.isArray(theme)) return { ok: false, reason: "`[theme]` must be a table of color tokens" };
  const keys = Object.keys(theme);
  if (keys.length > THEME_MAX_KEYS) return { ok: false, reason: `\`[theme]\` has ${keys.length} keys; the most is ${THEME_MAX_KEYS}` };
  for (const key of keys) {
    if (!THEME_KEYS.includes(key)) return { ok: false, reason: `\`[theme]\` has an unknown key \`${key}\`` };
    for (const dark of [false, true]) {
      const text = colorText(theme[key], dark);
      if (text === null) return { ok: false, reason: `\`[theme] ${key}\` must be a hex string or a table with \`light\` and \`dark\` hex strings` };
      if (parseHex(text) === null) return { ok: false, reason: `\`[theme] ${key}\` has the value "${text}"; use a hex color like "#1a2b3c"` };
    }
  }
  const accent = namedAccent && ACCENTS[namedAccent] ? namedAccent : DEFAULT_ACCENT;
  const out = {};
  for (const dark of [false, true]) {
    const p = defaultPalette(dark, accent);
    for (const key of keys) p[key] = parseHex(colorText(theme[key], dark));
    for (const [fg, bg, min] of PAIRS) {
      if (!Object.hasOwn(theme, fg) && !Object.hasOwn(theme, bg)) continue;
      const ratio = contrast(p[fg], p[bg]);
      if (ratio < min) return { ok: false, reason: `\`[theme]\` ${dark ? "dark" : "light"} mode: \`${fg}\` on \`${bg}\` has a contrast of ${ratio.toFixed(2)}:1; the least is ${min}:1` };
    }
    out[dark ? "dark" : "light"] = p;
  }
  return { ok: true, light: out.light, dark: out.dark };
}

const hex = (rgb) => `#${rgb.toString(16).padStart(6, "0")}`;

/** The CSS custom properties for a palette: `{ "--pl-bg": "#161618", ... }` (as `web_vars` in Rust). */
export function themeVars(p, dark) {
  const alpha = SELECTED_ALPHA[dark ? "dark" : "light"];
  const selected = p.selected === null ? `rgb(${(p.accent >> 16) & 0xff} ${(p.accent >> 8) & 0xff} ${p.accent & 0xff} / ${alpha})` : hex(p.selected);
  return {
    "--pl-bg": hex(p.background),
    "--pl-surface": hex(p.surface),
    "--pl-surface-alt": hex(p["surface-alt"]),
    "--pl-border": hex(p.border),
    "--pl-hover": hex(p.hover),
    "--pl-selected": selected,
    "--pl-text": hex(p.text),
    "--pl-muted": hex(p["text-muted"]),
    "--pl-accent": hex(p.accent),
    "--pl-accent-text": hex(p["on-accent"]),
    "--pl-accent-fg": hex(p.accent),
    "--pl-danger": hex(p.danger),
    "--pl-success": hex(p.success),
  };
}

/**
 * Reads, checks and resolves the theme of a manifest. Returns
 * `{ state: "none" }` (no `[theme]`), `{ state: "ok", light, dark }` (the
 * CSS variables of each appearance) or `{ state: "ignored", reason }`.
 */
export function resolveManifestTheme(manifestText) {
  const { accent, theme, error } = parseManifestTheme(manifestText);
  if (error) return { state: "ignored", reason: error };
  if (theme === null) return { state: "none" };
  const r = resolveOverride(theme, accent);
  if (!r.ok) return { state: "ignored", reason: r.reason };
  return { state: "ok", light: themeVars(r.light, false), dark: themeVars(r.dark, true) };
}

const declarations = (vars, scheme) =>
  `${Object.entries(vars).map(([k, v]) => `${k}: ${v};`).join(" ")} color-scheme: ${scheme}; background: var(--pl-bg); color: var(--pl-text);`;

/**
 * The CSS for the app root `selector`. Dark is the default, as in style.css;
 * light applies when the browser prefers light. Both rules are inside a
 * media query that does not match in high contrast or forced colors, so
 * the host tokens win there.
 */
export function themeCss(selector, resolved) {
  const normal = "(not (prefers-contrast: more)) and (not (forced-colors: active))";
  return (
    `@media ${normal} { ${selector} { ${declarations(resolved.dark, "dark")} } }\n` +
    `@media (prefers-color-scheme: light) and ${normal} { ${selector} { ${declarations(resolved.light, "light")} } }\n`
  );
}

/** The attribute that marks the themed app root, and the id of its `<style>`. */
export const THEME_ATTR = "data-pl-theme";
const STYLE_ID = "pl-app-theme";

/**
 * Applies the theme of `manifestText` to the app root `container` and to
 * nothing else: the host page, the Hub chrome, the consent UI and the
 * host banners keep the host theme. An override that fails a check is
 * ignored: `warn` gets a line and the host theme stays. Never throws.
 * Returns the state of `resolveManifestTheme`.
 */
export function applyAppTheme(container, manifestText, warn = (m) => console.warn(m)) {
  let result;
  try {
    result = resolveManifestTheme(manifestText);
  } catch (e) {
    result = { state: "ignored", reason: String(e?.message ?? e) };
  }
  const doc = container.ownerDocument;
  const old = doc?.getElementById?.(STYLE_ID);
  if (old) old.remove();
  container.removeAttribute(THEME_ATTR);
  if (result.state === "ignored") warn(`plinth: the app theme is ignored: ${result.reason}`);
  if (result.state !== "ok" || !doc) return result;
  const style = doc.createElement("style");
  style.id = STYLE_ID;
  style.textContent = themeCss(`[${THEME_ATTR}]`, result);
  (doc.head ?? doc.documentElement).appendChild(style);
  container.setAttribute(THEME_ATTR, "");
  return result;
}
