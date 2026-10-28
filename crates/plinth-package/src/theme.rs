//! Per-app theme token overrides (`docs/ui.md`, "Theming an app").
//!
//! An app can set the VALUES of the host color tokens in the `[theme]`
//! table of `plinth.toml`. The app code still uses token names only. The
//! table is data in the manifest, so it is in the signed package.
//!
//! This module has no UI dependency. It holds:
//!
//! - the host default palette and the named accents (`Palette::default_for`),
//!   which the desktop host (`plinth-ui`) and the tools both read;
//! - the override type, its checks (keys, hex, count) and the contrast guard;
//! - the merge of an override with the default palette (`ThemeOverride::resolve`);
//! - `web_vars`, the CSS custom properties that the web host sets. The web host
//!   (`web/theme.js`) has the same rules in JavaScript; `web/test/fixtures/
//!   theme-golden.json` is checked by a test on each side, so both hosts give
//!   the same colors for the same manifest.
//!
//! Derived tokens (the hover and selection tints, the backdrop) are not part
//! of an override: the host computes them from the merged base tokens.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The color tokens that an app can override, in the order of the docs.
pub const KEYS: [&str; 12] = [
    "background",
    "surface",
    "surface-alt",
    "border",
    "hover",
    "selected",
    "text",
    "text-muted",
    "accent",
    "on-accent",
    "danger",
    "success",
];

/// The most keys in a `[theme]` table (one for each token).
pub const MAX_KEYS: usize = 12;

/// The name of the accent when the app sets none.
pub const DEFAULT_ACCENT: &str = "teal";

/// The named accent palette. Each entry is (name, light, dark).
pub const ACCENTS: &[(&str, u32, u32)] = &[
    ("teal", 0x0b8a7e, 0x2ec4b6),
    ("blue", 0x1a63d6, 0x5b9cff),
    ("indigo", 0x4f46c8, 0x8c86ff),
    ("purple", 0x8a3ec9, 0xc08cff),
    ("pink", 0xc8327a, 0xff7ab8),
    ("red", 0xc93a2f, 0xff7a6e),
    ("orange", 0xc25a00, 0xff9f45),
    ("green", 0x2e8540, 0x5fd27a),
];

/// The opacity of the tint tokens that the host derives from the accent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alphas {
    /// `selected`, when the theme does not set it.
    pub selected: f64,
    pub selection: f64,
    pub accent_hover: f64,
    pub backdrop: f64,
}

impl Alphas {
    pub fn for_appearance(dark: bool) -> Alphas {
        if dark {
            Alphas { selected: 0.16, selection: 0.35, accent_hover: 0.85, backdrop: 0.55 }
        } else {
            Alphas { selected: 0.12, selection: 0.25, accent_hover: 0.88, backdrop: 0.35 }
        }
    }
}

/// The base color tokens of one appearance, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: u32,
    pub surface: u32,
    pub surface_alt: u32,
    pub border: u32,
    pub hover: u32,
    /// `None`: the host derives it from the accent (a tint).
    pub selected: Option<u32>,
    pub text: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub danger: u32,
    pub success: u32,
}

impl Palette {
    /// The host palette for an appearance and a named accent. An unknown
    /// accent name gives the default accent.
    pub fn default_for(dark: bool, accent: &str) -> Palette {
        let (_, light_accent, dark_accent) = ACCENTS.iter().find(|(name, _, _)| *name == accent).copied().unwrap_or(ACCENTS[0]);
        if dark {
            Palette {
                background: 0x161618,
                surface: 0x232326,
                surface_alt: 0x2e2e32,
                border: 0x37373c,
                hover: 0x34343a,
                selected: None,
                text: 0xf2f2f5,
                text_muted: 0x9a9aa2,
                accent: dark_accent,
                on_accent: 0x0b0b0c,
                danger: 0xff6b5e,
                success: 0x5fd27a,
            }
        } else {
            Palette {
                background: 0xf4f4f6,
                surface: 0xffffff,
                surface_alt: 0xececf0,
                border: 0xdedee4,
                hover: 0xe4e4ea,
                selected: None,
                text: 0x1b1b1f,
                text_muted: 0x6b6b74,
                accent: light_accent,
                on_accent: 0xffffff,
                danger: 0xc93a2f,
                success: 0x2e8540,
            }
        }
    }

    /// The color of a token by its key (`selected` is its blend over the
    /// surface when the theme does not set it).
    fn get(&self, key: &str, alphas: Alphas) -> u32 {
        match key {
            "background" => self.background,
            "surface" => self.surface,
            "surface-alt" => self.surface_alt,
            "border" => self.border,
            "hover" => self.hover,
            "selected" => self.selected.unwrap_or_else(|| blend(self.accent, self.surface, alphas.selected)),
            "text" => self.text,
            "text-muted" => self.text_muted,
            "accent" => self.accent,
            "on-accent" => self.on_accent,
            "danger" => self.danger,
            _ => self.success,
        }
    }

    fn set(&mut self, key: &str, value: u32) {
        match key {
            "background" => self.background = value,
            "surface" => self.surface = value,
            "surface-alt" => self.surface_alt = value,
            "border" => self.border = value,
            "hover" => self.hover = value,
            "selected" => self.selected = Some(value),
            "text" => self.text = value,
            "text-muted" => self.text_muted = value,
            "accent" => self.accent = value,
            "on-accent" => self.on_accent = value,
            "danger" => self.danger = value,
            _ => self.success = value,
        }
    }
}

/// One color of the override: one value for light and dark, or one for each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThemeColor {
    Same(String),
    Pair(ColorPair),
}

/// `{ light = "#rrggbb", dark = "#rrggbb" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorPair {
    pub light: String,
    pub dark: String,
}

impl ThemeColor {
    /// The text of the color for an appearance.
    fn text(&self, dark: bool) -> &str {
        match self {
            ThemeColor::Same(s) => s,
            ThemeColor::Pair(p) => {
                if dark {
                    &p.dark
                } else {
                    &p.light
                }
            }
        }
    }
}

/// The `[theme]` table: token key -> color. The type is data only; every
/// check runs in [`ThemeOverride::check`] and [`ThemeOverride::resolve`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ThemeOverride(pub BTreeMap<String, ThemeColor>);

/// Why an override is not valid. `Display` is the message for the author.
#[derive(Debug, Clone, PartialEq)]
pub enum ThemeError {
    NotATable,
    TooMany { count: usize },
    UnknownKey { key: String },
    BadHex { key: String, value: String },
    /// A value that is neither a hex string nor a `{ light, dark }` table.
    BadShape { key: String },
    Contrast { fg: &'static str, bg: &'static str, dark: bool, ratio: f64, min: f64 },
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ThemeError::NotATable => write!(f, "`[theme]` must be a table of color tokens"),
            ThemeError::TooMany { count } => write!(f, "`[theme]` has {count} keys; the most is {MAX_KEYS}"),
            ThemeError::UnknownKey { key } => write!(f, "`[theme]` has an unknown key `{key}` (known: {})", KEYS.join(", ")),
            ThemeError::BadHex { key, value } => write!(f, "`[theme] {key}` has the value \"{value}\"; use a hex color like \"#1a2b3c\""),
            ThemeError::BadShape { key } => {
                write!(f, "`[theme] {key}` must be a hex string or a table with `light` and `dark` hex strings")
            }
            ThemeError::Contrast { fg, bg, dark, ratio, min } => write!(
                f,
                "`[theme]` {} mode: `{fg}` on `{bg}` has a contrast of {ratio:.2}:1; the least is {min}:1",
                if *dark { "dark" } else { "light" }
            ),
        }
    }
}

impl std::error::Error for ThemeError {}

/// Parses exactly `#rrggbb` (hex digits in any case).
pub fn parse_hex(s: &str) -> Option<u32> {
    let digits = s.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// The WCAG 2 relative luminance of `0xRRGGBB`.
pub fn luminance(rgb: u32) -> f64 {
    let lin = |v: u32| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin((rgb >> 16) & 0xff) + 0.7152 * lin((rgb >> 8) & 0xff) + 0.0722 * lin(rgb & 0xff)
}

/// The WCAG 2 contrast ratio of two colors (1 to 21).
pub fn contrast(a: u32, b: u32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `top` with opacity `alpha` over `under`, rounded to a byte per channel.
fn blend(top: u32, under: u32, alpha: f64) -> u32 {
    let mix = |shift: u32| {
        let (t, u) = (f64::from((top >> shift) & 0xff), f64::from((under >> shift) & 0xff));
        ((t * alpha + u * (1.0 - alpha)).round() as u32) & 0xff
    };
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

/// The pairs that the contrast guard checks: (text token, background token,
/// least ratio). A pair is checked when the override sets one of its tokens.
const PAIRS: [(&str, &str, f64); 6] = [
    ("text", "background", 4.5),
    ("text", "surface", 4.5),
    ("on-accent", "accent", 4.5),
    ("text-muted", "background", 3.0),
    ("danger", "background", 3.0),
    ("success", "background", 3.0),
];

/// The merged palettes of both appearances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    pub light: Palette,
    pub dark: Palette,
}

impl Resolved {
    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }
}

impl ThemeOverride {
    /// Reads the `[theme]` value of `plinth.toml` with messages for the
    /// author (a wrong shape, an unknown key, a bad hex, too many keys).
    pub fn from_toml(value: &toml::Value) -> Result<ThemeOverride, ThemeError> {
        let table = value.as_table().ok_or(ThemeError::NotATable)?;
        if table.len() > MAX_KEYS {
            return Err(ThemeError::TooMany { count: table.len() });
        }
        let mut out = BTreeMap::new();
        for (key, v) in table {
            if !KEYS.contains(&key.as_str()) {
                return Err(ThemeError::UnknownKey { key: key.clone() });
            }
            let color = match v {
                toml::Value::String(s) => ThemeColor::Same(s.clone()),
                toml::Value::Table(t) => {
                    let (Some(toml::Value::String(light)), Some(toml::Value::String(dark)), 2) = (t.get("light"), t.get("dark"), t.len()) else {
                        return Err(ThemeError::BadShape { key: key.clone() });
                    };
                    ThemeColor::Pair(ColorPair { light: light.clone(), dark: dark.clone() })
                }
                _ => return Err(ThemeError::BadShape { key: key.clone() }),
            };
            out.insert(key.clone(), color);
        }
        let theme = ThemeOverride(out);
        theme.check()?;
        Ok(theme)
    }

    /// The shape rules: at most 12 keys, known keys only, hex colors only.
    /// A host runs this on every package (a package is never trusted).
    pub fn check(&self) -> Result<(), ThemeError> {
        if self.0.len() > MAX_KEYS {
            return Err(ThemeError::TooMany { count: self.0.len() });
        }
        for (key, color) in &self.0 {
            if !KEYS.contains(&key.as_str()) {
                return Err(ThemeError::UnknownKey { key: key.clone() });
            }
            for dark in [false, true] {
                let text = color.text(dark);
                if parse_hex(text).is_none() {
                    return Err(ThemeError::BadHex { key: key.clone(), value: text.to_owned() });
                }
            }
        }
        Ok(())
    }

    /// Merges the override with the host palette for both appearances and
    /// runs the contrast guard. `named_accent` is the app's `accent` (the
    /// default when `None`); `[theme].accent` wins over it.
    ///
    /// A pair is checked when the override sets at least one of its two
    /// tokens in that appearance. A pair that the app does not touch keeps
    /// the host palette, which the author did not choose (the default
    /// light teal accent with white text is 4.2:1, for example).
    pub fn resolve(&self, named_accent: Option<&str>) -> Result<Resolved, ThemeError> {
        self.check()?;
        let accent = named_accent.unwrap_or(DEFAULT_ACCENT);
        let merge = |dark: bool| -> Result<Palette, ThemeError> {
            let mut p = Palette::default_for(dark, accent);
            for (key, color) in &self.0 {
                // `check` made sure that the text is a valid hex.
                let value = parse_hex(color.text(dark)).ok_or_else(|| ThemeError::BadHex { key: key.clone(), value: color.text(dark).to_owned() })?;
                p.set(key, value);
            }
            let alphas = Alphas::for_appearance(dark);
            for (fg, bg, min) in PAIRS {
                if !self.0.contains_key(fg) && !self.0.contains_key(bg) {
                    continue;
                }
                let ratio = contrast(p.get(fg, alphas), p.get(bg, alphas));
                if ratio < min {
                    return Err(ThemeError::Contrast { fg, bg, dark, ratio, min });
                }
            }
            Ok(p)
        };
        Ok(Resolved { light: merge(false)?, dark: merge(true)? })
    }
}

fn hex(rgb: u32) -> String {
    format!("#{rgb:06x}")
}

fn rgba(rgb: u32, alpha: f64) -> String {
    format!("rgb({} {} {} / {})", (rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff, alpha)
}

/// The CSS custom properties that the web host sets for a palette
/// (`web/style.css`, `web/theme.js`). `accent-fg` is the accent itself: the
/// desktop host paints accent text in the accent token too.
pub fn web_vars(p: &Palette, dark: bool) -> Vec<(&'static str, String)> {
    let alphas = Alphas::for_appearance(dark);
    vec![
        ("--pl-bg", hex(p.background)),
        ("--pl-surface", hex(p.surface)),
        ("--pl-surface-alt", hex(p.surface_alt)),
        ("--pl-border", hex(p.border)),
        ("--pl-hover", hex(p.hover)),
        ("--pl-selected", p.selected.map_or_else(|| rgba(p.accent, alphas.selected), hex)),
        ("--pl-text", hex(p.text)),
        ("--pl-muted", hex(p.text_muted)),
        ("--pl-accent", hex(p.accent)),
        ("--pl-accent-text", hex(p.on_accent)),
        ("--pl-accent-fg", hex(p.accent)),
        ("--pl-danger", hex(p.danger)),
        ("--pl-success", hex(p.success)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<ThemeOverride, ThemeError> {
        let v: toml::Value = toml::from_str(text).unwrap();
        ThemeOverride::from_toml(v.get("theme").unwrap())
    }

    #[test]
    fn parses_hex_and_light_dark_tables() {
        let t = parse("[theme]\nbackground = \"#101820\"\n[theme.accent]\nlight = \"#0a5cd6\"\ndark = \"#6ea8ff\"\n").unwrap();
        assert_eq!(t.0["background"], ThemeColor::Same("#101820".into()));
        assert_eq!(t.0["accent"], ThemeColor::Pair(ColorPair { light: "#0a5cd6".into(), dark: "#6ea8ff".into() }));
        // Inline tables work too.
        let t = parse("[theme]\naccent = { light = \"#0a5cd6\", dark = \"#6ea8ff\" }\n").unwrap();
        assert!(matches!(t.0["accent"], ThemeColor::Pair(_)));
    }

    #[test]
    fn rejects_bad_hex() {
        for bad in ["red", "#fff", "#12345", "#1234567", "#gg0000", "123456", "rgb(1,2,3)", " #123456"] {
            let e = parse(&format!("[theme]\ntext = \"{bad}\"\n")).unwrap_err();
            assert!(matches!(e, ThemeError::BadHex { .. }), "{bad}: {e}");
        }
        // A bad value in one half of a pair.
        let e = parse("[theme]\ntext = { light = \"#000000\", dark = \"white\" }\n").unwrap_err();
        assert!(e.to_string().contains("white"), "{e}");
        assert!(parse("[theme]\ntext = \"#AbCdEf\"\n").is_ok());
    }

    #[test]
    fn rejects_unknown_keys_and_shapes() {
        let e = parse("[theme]\nbrand = \"#112233\"\n").unwrap_err();
        assert_eq!(e, ThemeError::UnknownKey { key: "brand".into() });
        assert!(e.to_string().contains("unknown key `brand`") && e.to_string().contains("surface-alt"));
        assert!(matches!(parse("[theme]\ntext = 5\n"), Err(ThemeError::BadShape { .. })));
        assert!(matches!(parse("[theme]\ntext = { light = \"#000000\" }\n"), Err(ThemeError::BadShape { .. })));
        assert!(matches!(parse("[theme]\ntext = { light = \"#000000\", dark = \"#ffffff\", x = \"#ffffff\" }\n"), Err(ThemeError::BadShape { .. })));
        assert_eq!(ThemeOverride::from_toml(&toml::Value::Integer(1)), Err(ThemeError::NotATable));
    }

    #[test]
    fn rejects_too_many_keys() {
        // 12 known keys are fine; a 13th key (any name) is "too many".
        let all: String = KEYS.iter().map(|k| format!("{k} = \"#808080\"\n")).collect();
        assert!(parse(&format!("[theme]\n{all}")).is_ok());
        let e = parse(&format!("[theme]\n{all}extra = \"#808080\"\n")).unwrap_err();
        assert_eq!(e, ThemeError::TooMany { count: 13 });
        // The same rule for data that came from a package.
        let mut t = ThemeOverride::default();
        for i in 0..13 {
            t.0.insert(format!("k{i}"), ThemeColor::Same("#000000".into()));
        }
        assert_eq!(t.check(), Err(ThemeError::TooMany { count: 13 }));
    }

    #[test]
    fn host_defaults_pass_the_text_guard() {
        // The pairs that the guard checks, on the host palette itself. The
        // light on-accent/accent pairs of some accents are below 4.5:1.
        for dark in [false, true] {
            for (name, _, _) in ACCENTS {
                let p = Palette::default_for(dark, name);
                for (fg, bg, min) in PAIRS.into_iter().filter(|(fg, _, _)| *fg != "on-accent") {
                    let a = Alphas::for_appearance(dark);
                    let r = contrast(p.get(fg, a), p.get(bg, a));
                    assert!(r >= min, "{name} dark {dark}: {fg}/{bg} {r:.2}");
                }
            }
        }
    }

    fn ov(pairs: &[(&str, &str)]) -> ThemeOverride {
        ThemeOverride(pairs.iter().map(|(k, v)| ((*k).to_owned(), ThemeColor::Same((*v).to_owned()))).collect())
    }

    fn pair(light: &str, dark: &str) -> ThemeColor {
        ThemeColor::Pair(ColorPair { light: light.into(), dark: dark.into() })
    }

    /// A blue accent with its own text color, for light and dark.
    fn brand_accent() -> ThemeOverride {
        let mut t = ThemeOverride::default();
        t.0.insert("accent".into(), pair("#0b4fa8", "#6ea8ff"));
        t.0.insert("on-accent".into(), pair("#ffffff", "#0b0b0c"));
        t
    }

    #[test]
    fn contrast_math() {
        assert!((contrast(0x000000, 0xffffff) - 21.0).abs() < 0.001);
        assert!((contrast(0x777777, 0xffffff) - 4.48).abs() < 0.01);
        assert!((contrast(0x123456, 0x123456) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn contrast_guard_passes_and_fails() {
        // A good brand theme (the same colors in light and dark).
        // Setting a background also checks the muted text, danger and
        // success on it, so a full light theme sets those too.
        let good = ov(&[
            ("background", "#fffaf0"),
            ("surface", "#ffffff"),
            ("text", "#1a1a1a"),
            ("text-muted", "#555555"),
            ("danger", "#b00020"),
            ("success", "#1b6e2e"),
            ("accent", "#0b4fa8"),
            ("on-accent", "#ffffff"),
        ]);
        assert!(good.resolve(None).is_ok());
        // Only the background: the host dark text is too faint on it. The
        // first failure names the pair.
        let e = ov(&[("background", "#fffaf0")]).resolve(None).unwrap_err();
        assert!(matches!(e, ThemeError::Contrast { dark: true, .. }), "{e}");
        // Light text on a light background fails: names the pair and ratio.
        let bad = ov(&[("text", "#eeeeee")]);
        let e = bad.resolve(None).unwrap_err();
        match &e {
            ThemeError::Contrast { fg: "text", bg: "background", dark: false, ratio, min } => {
                assert!(*ratio < 4.5 && *min == 4.5);
            }
            other => panic!("{other:?}"),
        }
        let msg = e.to_string();
        assert!(msg.contains("`text` on `background`") && msg.contains("light") && msg.contains("4.5:1"), "{msg}");
    }

    #[test]
    fn contrast_guard_checks_each_appearance() {
        // Good in light (dark text on light), bad in dark (dark text on the
        // dark host background).
        let mut t = ThemeOverride::default();
        t.0.insert("text".into(), ThemeColor::Pair(ColorPair { light: "#101010".into(), dark: "#202020".into() }));
        match t.resolve(None).unwrap_err() {
            ThemeError::Contrast { dark: true, fg: "text", .. } => {}
            other => panic!("{other:?}"),
        }
        t.0.insert("text".into(), ThemeColor::Pair(ColorPair { light: "#101010".into(), dark: "#f0f0f0".into() }));
        assert!(t.resolve(None).is_ok());
    }

    #[test]
    fn each_guarded_pair_is_checked() {
        // (override, expected pair)
        let cases: [(&[(&str, &str)], &str, &str); 6] = [
            (&[("text", "#f4f4f6")], "text", "background"),
            (&[("surface", "#1b1b1f")], "text", "surface"),
            (&[("accent", "#ffffff")], "on-accent", "accent"),
            (&[("text-muted", "#f0f0f0")], "text-muted", "background"),
            (&[("danger", "#f4f4f6")], "danger", "background"),
            (&[("success", "#f4f4f6")], "success", "background"),
        ];
        for (pairs, fg, bg) in cases {
            // Both appearances take the same value, so one of them fails.
            let e = ov(pairs).resolve(None).unwrap_err();
            let ThemeError::Contrast { fg: f, bg: b, .. } = e else { panic!("{pairs:?}: {e}") };
            assert_eq!((f, b), (fg, bg), "{pairs:?}");
        }
    }

    #[test]
    fn untouched_pairs_are_not_blamed() {
        // Light teal with white is 4.2:1 on the host palette. An override
        // that does not touch that pair still passes.
        assert!(contrast(0x0b8a7e, 0xffffff) < 4.5);
        assert!(ov(&[("hover", "#e0e0ff")]).resolve(None).is_ok());
        assert!(ov(&[("border", "#999999")]).resolve(Some("orange")).is_ok());
    }

    #[test]
    fn merge_override_wins_and_the_rest_is_the_host_palette() {
        let r = ov(&[("hover", "#e0e0ff"), ("border", "#999999")]).resolve(Some("pink")).unwrap();
        let host = Palette::default_for(false, "pink");
        assert_eq!(r.light.hover, 0xe0e0ff);
        assert_eq!(r.light.border, 0x999999);
        assert_eq!(r.light.background, host.background);
        assert_eq!(r.light.accent, host.accent);
        // Not overridden: the host palette (with the named accent's rest).
        assert_eq!(r.light.surface, host.surface);
        assert_eq!(r.light.text, host.text);
        assert_eq!(r.dark.surface, Palette::default_for(true, "pink").surface);
        // Same value in both appearances.
        assert_eq!(r.dark.hover, 0xe0e0ff);
    }

    #[test]
    fn theme_accent_wins_over_the_named_accent_and_tints_follow_it() {
        let r = brand_accent().resolve(Some("pink")).unwrap();
        assert_eq!(r.light.accent, 0x0b4fa8);
        assert_eq!(r.dark.accent, 0x6ea8ff);
        assert_eq!(r.light.selected, None, "selected stays derived from the new accent");
        let vars = web_vars(&r.light, false);
        let selected = &vars.iter().find(|(k, _)| *k == "--pl-selected").unwrap().1;
        assert_eq!(selected, "rgb(11 79 168 / 0.12)");
        // Without an override of `accent`, the named accent is used.
        let r = ov(&[("hover", "#e0e0ff")]).resolve(Some("pink")).unwrap();
        assert_eq!(r.light.accent, 0xc8327a);
        // An explicit `selected` is used as it is.
        let r = ov(&[("selected", "#ccddee")]).resolve(None).unwrap();
        assert_eq!(r.light.selected, Some(0xccddee));
    }

    /// The default palette is the one that the desktop themes always had.
    #[test]
    fn host_palette_values_are_stable() {
        let p = Palette::default_for(true, "teal");
        assert_eq!((p.background, p.text, p.accent, p.on_accent), (0x161618, 0xf2f2f5, 0x2ec4b6, 0x0b0b0c));
        let p = Palette::default_for(false, "blue");
        assert_eq!((p.background, p.text, p.accent, p.on_accent), (0xf4f4f6, 0x1b1b1f, 0x1a63d6, 0xffffff));
        assert_eq!(Palette::default_for(false, "nope"), Palette::default_for(false, "teal"));
    }
}
