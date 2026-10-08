//! Runtime-owned theme tokens (SPEC.md §6.5) and width classes (SPEC.md §6.1).

use gpui::{Hsla, Pixels, WindowAppearance, hsla, hsla_to_rgba, px, rgb, rgb_to_hsla};

/// The window width class. Compact < 600 px ≤ regular < 1200 px ≤ wide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthClass {
    Compact,
    Regular,
    Wide,
}

impl WidthClass {
    pub fn from_width(width: Pixels) -> Self {
        if width < px(600.) {
            WidthClass::Compact
        } else if width < px(1200.) {
            WidthClass::Regular
        } else {
            WidthClass::Wide
        }
    }
}

/// The color tokens for one appearance.
#[derive(Clone, Copy)]
pub struct Tokens {
    pub background: Hsla,
    pub surface: Hsla,
    pub surface_alt: Hsla,
    pub border: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub selection: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub on_accent: Hsla,
    pub danger: Hsla,
    pub success: Hsla,
    pub track: Hsla,
    pub knob: Hsla,
    /// The backdrop behind a modal `Sheet`/`Dialog` (UI API 1.2).
    pub backdrop: Hsla,
    /// The width of the focus ring, in pixels (stronger in high contrast).
    pub focus_ring: f32,
    /// `true` when the system asks for less motion: the host shows no
    /// transitions and no caret blink (PRODUCTION U7).
    pub reduced_motion: bool,
}

/// The named accent palette. Each entry is (name, light, dark).
const ACCENTS: &[(&str, u32, u32)] = &[
    ("teal", 0x0b8a7e, 0x2ec4b6),
    ("blue", 0x1a63d6, 0x5b9cff),
    ("indigo", 0x4f46c8, 0x8c86ff),
    ("purple", 0x8a3ec9, 0xc08cff),
    ("pink", 0xc8327a, 0xff7ab8),
    ("red", 0xc93a2f, 0xff7a6e),
    ("orange", 0xc25a00, 0xff9f45),
    ("green", 0x2e8540, 0x5fd27a),
];

fn c(hex: u32) -> Hsla {
    rgb_to_hsla(rgb(hex))
}

pub fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
    let mut color = color;
    color.alpha = alpha;
    color
}

impl Tokens {
    pub fn new(appearance: WindowAppearance, accent: &str) -> Self {
        let dark = matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark);
        let (_, light_accent, dark_accent) =
            ACCENTS.iter().find(|(name, _, _)| *name == accent).copied().unwrap_or(ACCENTS[0]);
        if dark {
            let accent = c(dark_accent);
            Self {
                background: c(0x161618),
                surface: c(0x232326),
                surface_alt: c(0x2e2e32),
                border: c(0x37373c),
                hover: c(0x34343a),
                selected: with_alpha(accent, 0.16),
                selection: with_alpha(accent, 0.35),
                text: c(0xf2f2f5),
                text_muted: c(0x9a9aa2),
                accent,
                accent_hover: with_alpha(accent, 0.85),
                on_accent: c(0x0b0b0c),
                danger: c(0xff6b5e),
                success: c(0x5fd27a),
                track: c(0x48484f),
                knob: c(0xffffff),
                backdrop: with_alpha(c(0x000000), 0.55),
                focus_ring: FOCUS_RING,
                reduced_motion: false,
            }
        } else {
            let accent = c(light_accent);
            Self {
                background: c(0xf4f4f6),
                surface: c(0xffffff),
                surface_alt: c(0xececf0),
                border: c(0xdedee4),
                hover: c(0xe4e4ea),
                selected: with_alpha(accent, 0.12),
                selection: with_alpha(accent, 0.25),
                text: c(0x1b1b1f),
                text_muted: c(0x6b6b74),
                accent,
                accent_hover: with_alpha(accent, 0.88),
                on_accent: c(0xffffff),
                danger: c(0xc93a2f),
                success: c(0x2e8540),
                track: c(0xc9c9d0),
                knob: c(0xffffff),
                backdrop: with_alpha(c(0x000000), 0.35),
                focus_ring: FOCUS_RING,
                reduced_motion: false,
            }
        }
    }
}

/// The focus ring width, in pixels, of the normal theme. The web host draws
/// a 2 px outline in the accent color (`web/style.css`, `:focus-visible`).
pub const FOCUS_RING: f32 = 2.0;
/// The focus ring width in high contrast (3 px, as `web/style.css`).
pub const FOCUS_RING_HIGH_CONTRAST: f32 = 3.0;
/// The minimum contrast of text in the high-contrast theme (WCAG 1.4.6, AAA).
pub const HIGH_CONTRAST_RATIO: f32 = 7.0;
/// The range of the text scale (the Windows "Text size" setting is 100 % to 225 %).
pub const TEXT_SCALE_RANGE: (f32, f32) = (1.0, 2.25);
/// The rem size of the normal text scale (gpui's default).
pub const BASE_REM: f32 = 16.0;

/// The system colors of a high-contrast theme that the OS gives (Windows
/// `GetSysColor`). With them, the high-contrast tokens use the user's colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemColors {
    /// The window background (`COLOR_WINDOW`).
    pub window: Hsla,
    /// The window text (`COLOR_WINDOWTEXT`).
    pub window_text: Hsla,
    /// The background of selected items (`COLOR_HIGHLIGHT`).
    pub highlight: Hsla,
    /// The text of selected items (`COLOR_HIGHLIGHTTEXT`).
    pub highlight_text: Hsla,
    /// The face of buttons (`COLOR_BTNFACE`).
    pub button_face: Hsla,
}

/// The OS settings that the runtime theme follows (SPEC.md §6.5,
/// PRODUCTION U7). The desktop host reads them from the OS, applies the
/// `PLINTH_*` overrides and keeps this global up to date
/// (`plinth_host_desktop::system_settings`). With no global (the headless
/// tests), the defaults apply: the window appearance, normal contrast,
/// 100 % text and full motion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemSettings {
    /// `Some(true)` forces dark, `Some(false)` forces light. `None` follows
    /// the window appearance (gpui-ce reports the OS setting and its changes).
    pub dark: Option<bool>,
    pub high_contrast: bool,
    /// The OS high-contrast colors, when the OS gives them.
    pub system_colors: Option<SystemColors>,
    /// 1.0 is 100 %. Clamped to `TEXT_SCALE_RANGE`.
    pub text_scale: f32,
    pub reduced_motion: bool,
}

impl Default for SystemSettings {
    fn default() -> Self {
        Self { dark: None, high_contrast: false, system_colors: None, text_scale: 1.0, reduced_motion: false }
    }
}

impl gpui::Global for SystemSettings {}

/// Parses a boolean override: `1`, `true`, `on`, `yes` are true; `0`,
/// `false`, `off`, `no` are false; anything else is `None` (no override).
fn parse_flag(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

impl SystemSettings {
    /// Applies the test and user overrides: `PLINTH_THEME=dark|light`,
    /// `PLINTH_HIGH_CONTRAST=1|0`, `PLINTH_TEXT_SCALE=2.0` (or `200%`),
    /// `PLINTH_REDUCED_MOTION=1|0`. An override wins over the OS setting.
    pub fn with_overrides(self, var: impl Fn(&str) -> Option<String>) -> Self {
        let mut s = self;
        match var("PLINTH_THEME").as_deref().map(str::trim) {
            Some("dark") => s.dark = Some(true),
            Some("light") => s.dark = Some(false),
            Some("system") => s.dark = None,
            _ => {}
        }
        if let Some(on) = var("PLINTH_HIGH_CONTRAST").as_deref().and_then(parse_flag) {
            s.high_contrast = on;
            if !on {
                s.system_colors = None;
            }
        }
        if let Some(scale) = var("PLINTH_TEXT_SCALE").as_deref().and_then(parse_scale) {
            s.text_scale = scale;
        }
        if let Some(on) = var("PLINTH_REDUCED_MOTION").as_deref().and_then(parse_flag) {
            s.reduced_motion = on;
        }
        s.text_scale = clamp_scale(s.text_scale);
        s
    }

    /// [`Self::with_overrides`] from the process environment.
    pub fn with_env_overrides(self) -> Self {
        self.with_overrides(|name| std::env::var(name).ok())
    }

    /// Whether the theme is dark, from the override or the window appearance.
    /// In high contrast with the OS colors, the darkness of the OS window color.
    pub fn is_dark(&self, appearance: WindowAppearance) -> bool {
        if let Some(dark) = self.dark {
            return dark;
        }
        if self.high_contrast
            && let Some(colors) = self.system_colors
        {
            return relative_luminance(colors.window) < 0.18;
        }
        matches!(appearance, WindowAppearance::Dark | WindowAppearance::VibrantDark)
    }

    /// The window's rem size: the text scale of the runtime type scale.
    pub fn rem_size(&self) -> Pixels {
        px(BASE_REM * clamp_scale(self.text_scale))
    }
}

/// Parses `2`, `2.0`, `200%` or `200` (a percent when above 10).
fn parse_scale(value: &str) -> Option<f32> {
    let v = value.trim();
    let (num, percent) = match v.strip_suffix('%') {
        Some(n) => (n.trim(), true),
        None => (v, false),
    };
    let n: f32 = num.parse().ok().filter(|n: &f32| n.is_finite() && *n > 0.0)?;
    Some(if percent || n > 10.0 { n / 100.0 } else { n })
}

fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() { scale.clamp(TEXT_SCALE_RANGE.0, TEXT_SCALE_RANGE.1) } else { 1.0 }
}

/// The WCAG 2 relative luminance of a color (its alpha is ignored).
pub fn relative_luminance(color: Hsla) -> f32 {
    let c = hsla_to_rgba(color).color;
    let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.red) + 0.7152 * lin(c.green) + 0.0722 * lin(c.blue)
}

/// The WCAG 2 contrast ratio of two opaque colors (1 to 21).
pub fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// A color with alpha painted over an opaque background.
pub fn blend(top: Hsla, under: Hsla) -> Hsla {
    let (t, u) = (hsla_to_rgba(top).color, hsla_to_rgba(under).color);
    let a = top.alpha;
    let mix = |x: f32, y: f32| x * a + y * (1.0 - a);
    rgb_to_hsla(gpui::Rgba::new(mix(t.red, u.red), mix(t.green, u.green), mix(t.blue, u.blue), 1.0))
}

/// Moves the lightness of `color` away from `background` until the
/// contrast is at least `ratio` (or the lightness is at its end).
fn with_contrast(color: Hsla, background: Hsla, ratio: f32) -> Hsla {
    let lighter = relative_luminance(background) < 0.18;
    let mut out = color;
    for _ in 0..100 {
        if contrast_ratio(out, background) >= ratio {
            break;
        }
        let l = out.color.lightness + if lighter { 0.01 } else { -0.01 };
        out = hsla(out.color.hue.into_positive_degrees() / 360.0, out.color.saturation, l.clamp(0.0, 1.0), 1.0);
    }
    out
}

impl Tokens {
    /// The tokens for the window appearance and the OS settings.
    pub fn themed(appearance: WindowAppearance, settings: &SystemSettings, accent: &str) -> Self {
        let dark = settings.is_dark(appearance);
        let mut t = if settings.high_contrast {
            Self::high_contrast(dark, settings.system_colors, accent)
        } else {
            Self::new(if dark { WindowAppearance::Dark } else { WindowAppearance::Light }, accent)
        };
        t.reduced_motion = settings.reduced_motion;
        t
    }

    /// The high-contrast theme: text and its background at least 7:1, the
    /// borders in the text color, no faint muted text, a 3 px focus ring.
    /// With the OS colors, they win; the built-in colors otherwise.
    pub fn high_contrast(dark: bool, system: Option<SystemColors>, accent: &str) -> Self {
        let (_, light_accent, dark_accent) =
            ACCENTS.iter().find(|(name, _, _)| *name == accent).copied().unwrap_or(ACCENTS[0]);
        let mut t = if dark {
            let background = c(0x000000);
            let accent = with_contrast(c(dark_accent), background, HIGH_CONTRAST_RATIO);
            Self {
                background,
                surface: c(0x000000),
                surface_alt: c(0x141414),
                border: c(0xffffff),
                hover: c(0x2a2a2a),
                selected: with_alpha(accent, 0.22),
                selection: with_alpha(accent, 0.45),
                text: c(0xffffff),
                text_muted: c(0xe6e6e6),
                accent,
                accent_hover: accent,
                on_accent: c(0x000000),
                danger: c(0xff9e94),
                success: c(0x7ee787),
                track: c(0xb0b0b0),
                knob: c(0x000000),
                backdrop: with_alpha(c(0x000000), 0.75),
                focus_ring: FOCUS_RING_HIGH_CONTRAST,
                reduced_motion: false,
            }
        } else {
            let background = c(0xffffff);
            let accent = with_contrast(c(light_accent), background, HIGH_CONTRAST_RATIO);
            Self {
                background,
                surface: c(0xffffff),
                surface_alt: c(0xf0f0f0),
                border: c(0x000000),
                hover: c(0xe0e0e0),
                selected: with_alpha(accent, 0.14),
                selection: with_alpha(accent, 0.3),
                text: c(0x000000),
                text_muted: c(0x1f1f1f),
                accent,
                accent_hover: accent,
                on_accent: c(0xffffff),
                danger: c(0x9b0000),
                success: c(0x0b5d1e),
                track: c(0x595959),
                knob: c(0xffffff),
                backdrop: with_alpha(c(0x000000), 0.6),
                focus_ring: FOCUS_RING_HIGH_CONTRAST,
                reduced_motion: false,
            }
        };
        if let Some(s) = system {
            t.background = s.window;
            t.surface = s.window;
            t.surface_alt = s.button_face;
            t.border = s.window_text;
            t.text = s.window_text;
            t.text_muted = s.window_text;
            t.accent = s.highlight;
            t.accent_hover = s.highlight;
            t.on_accent = s.highlight_text;
            t.selected = with_alpha(s.highlight, 0.3);
            t.selection = with_alpha(s.highlight, 0.5);
            t.knob = s.highlight_text;
        }
        t
    }
}

impl Tokens {
    /// The theme's chart palette (SPEC.md §6.3, UI API 1.5): colors derived
    /// from the app's accent and the neutral tokens, distinguishable from
    /// each other in both light and dark. Apps never pick chart colors.
    pub fn chart_palette(&self) -> [Hsla; 6] {
        let a = self.accent;
        let h = a.color.hue.into_positive_degrees() / 360.0;
        let s = a.color.saturation;
        let l = a.color.lightness;
        let shift = |d: f32| (h + d).rem_euclid(1.0);
        [
            a,
            hsla(shift(0.5), s, l, 1.0),
            hsla(shift(0.12), s, l, 1.0),
            hsla(shift(0.62), s * 0.8, (l + 0.1).min(0.85), 1.0),
            hsla(shift(0.3), s * 0.75, l, 1.0),
            self.text_muted,
        ]
    }
}

/// Maps a runtime icon name to a glyph. M0 uses text glyphs; the real icon
/// set comes with UI API 1.0 in M2.
pub fn icon_glyph(name: &str) -> &'static str {
    match name {
        "house" | "home" => "⌂",
        "gear" | "settings" => "⚙",
        "check" => "✓",
        "list" => "≡",
        "plus" | "add" => "+",
        "trash" | "delete" => "✕",
        "star" => "★",
        "info" => "ⓘ",
        "number" | "counter" => "#",
        // UI API 1.2
        "search" => "🔍",
        "edit" => "✍",
        "close" => "✕",
        "back" => "←",
        "forward" => "→",
        "calendar" => "📅",
        "clock" => "🕐",
        "user" => "👤",
        "mail" => "✉",
        "heart" => "♥",
        "bell" => "🔔",
        "share" => "⤴",
        "download" => "⬇",
        "upload" => "⬆",
        "refresh" => "↻",
        "filter" => "▾",
        "menu" => "≡",
        "more" => "…",
        "lock" => "🔒",
        "warning" => "⚠",
        _ => "•",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> impl Iterator<Item = &'static str> {
        ACCENTS.iter().map(|(name, _, _)| *name)
    }

    /// PRODUCTION U7: each text/background pair of the high-contrast theme
    /// has a WCAG contrast of at least 7:1, for each accent, light and dark.
    #[test]
    fn high_contrast_pairs_are_at_least_7_to_1() {
        for dark in [false, true] {
            for accent in names() {
                let t = Tokens::high_contrast(dark, None, accent);
                let selected = blend(t.selected, t.surface);
                let pairs = [
                    ("text/background", t.text, t.background),
                    ("text/surface", t.text, t.surface),
                    ("text/surface_alt", t.text, t.surface_alt),
                    ("text/hover", t.text, t.hover),
                    ("text/selected", t.text, selected),
                    ("text_muted/background", t.text_muted, t.background),
                    ("text_muted/surface", t.text_muted, t.surface),
                    ("text_muted/surface_alt", t.text_muted, t.surface_alt),
                    ("on_accent/accent", t.on_accent, t.accent),
                    ("accent/background", t.accent, t.background),
                    ("accent/surface", t.accent, t.surface),
                    ("danger/background", t.danger, t.background),
                    ("danger/surface", t.danger, t.surface),
                    ("success/background", t.success, t.background),
                    ("border/background", t.border, t.background),
                    ("knob/accent", t.knob, t.accent),
                ];
                for (name, fg, bg) in pairs {
                    let ratio = contrast_ratio(fg, bg);
                    assert!(ratio >= HIGH_CONTRAST_RATIO, "{name} is {ratio:.2}:1 (dark {dark}, accent {accent})");
                }
                assert_eq!(t.focus_ring, FOCUS_RING_HIGH_CONTRAST);
                // Non-text: the off track of a toggle at least 3:1 (WCAG 1.4.11).
                assert!(contrast_ratio(t.track, t.background) >= 3.0, "track (dark {dark})");
            }
        }
    }

    /// The normal themes keep body text at least 4.5:1 (WCAG AA); the
    /// high-contrast theme is stronger than the normal one.
    #[test]
    fn normal_themes_meet_aa_and_high_contrast_is_stronger() {
        for appearance in [WindowAppearance::Light, WindowAppearance::Dark] {
            let t = Tokens::new(appearance, "teal");
            assert!(contrast_ratio(t.text, t.background) >= 4.5);
            assert!(contrast_ratio(t.text_muted, t.surface) >= 4.5, "muted text {appearance:?}");
            let hc = Tokens::high_contrast(appearance == WindowAppearance::Dark, None, "teal");
            assert!(contrast_ratio(hc.text_muted, hc.surface) > contrast_ratio(t.text_muted, t.surface));
            assert!(contrast_ratio(hc.border, hc.background) > contrast_ratio(t.border, t.background));
        }
    }

    #[test]
    fn contrast_math_matches_wcag() {
        assert!((contrast_ratio(c(0x000000), c(0xffffff)) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(c(0x777777), c(0xffffff)) - 4.48).abs() < 0.02);
        assert!((contrast_ratio(c(0xffffff), c(0xffffff)) - 1.0).abs() < 0.001);
    }

    #[test]
    fn overrides_force_each_setting() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
        };
        let base = SystemSettings::default();
        assert_eq!(base.with_overrides(env(&[])), base);
        let s = base.with_overrides(env(&[
            ("PLINTH_THEME", "dark"),
            ("PLINTH_HIGH_CONTRAST", "1"),
            ("PLINTH_TEXT_SCALE", "200%"),
            ("PLINTH_REDUCED_MOTION", "true"),
        ]));
        assert_eq!(s.dark, Some(true));
        assert!(s.high_contrast && s.reduced_motion);
        assert_eq!(s.text_scale, 2.0);
        assert_eq!(s.rem_size(), px(32.));
        // The OS says on; the override says off.
        let os = SystemSettings { high_contrast: true, reduced_motion: true, text_scale: 1.5, ..base };
        let s = os.with_overrides(env(&[("PLINTH_HIGH_CONTRAST", "0"), ("PLINTH_REDUCED_MOTION", "0"), ("PLINTH_TEXT_SCALE", "1")]));
        assert!(!s.high_contrast && !s.reduced_motion);
        assert_eq!(s.text_scale, 1.0);
        // Out of range and bad values.
        assert_eq!(base.with_overrides(env(&[("PLINTH_TEXT_SCALE", "9")])).text_scale, 2.25);
        assert_eq!(base.with_overrides(env(&[("PLINTH_TEXT_SCALE", "50%")])).text_scale, 1.0);
        assert_eq!(base.with_overrides(env(&[("PLINTH_TEXT_SCALE", "big")])).text_scale, 1.0);
        assert_eq!(base.with_overrides(env(&[("PLINTH_THEME", "blue")])).dark, None);
    }

    #[test]
    fn theme_follows_the_settings() {
        let light = WindowAppearance::Light;
        let base = SystemSettings::default();
        // No override: the window appearance.
        assert_eq!(Tokens::themed(WindowAppearance::Dark, &base, "teal").background, Tokens::new(WindowAppearance::Dark, "teal").background);
        assert_eq!(Tokens::themed(light, &base, "teal").background, Tokens::new(light, "teal").background);
        // The override wins over the window appearance.
        let dark = SystemSettings { dark: Some(true), ..base };
        assert_eq!(Tokens::themed(light, &dark, "teal").background, Tokens::new(WindowAppearance::Dark, "teal").background);
        // High contrast.
        let hc = SystemSettings { high_contrast: true, ..base };
        let t = Tokens::themed(light, &hc, "teal");
        assert_eq!(t.background, c(0xffffff));
        assert_eq!(t.border, c(0x000000));
        // The OS colors win, and their window color picks dark or light.
        let colors = SystemColors {
            window: c(0x000000),
            window_text: c(0xffff00),
            highlight: c(0x1aebff),
            highlight_text: c(0x000000),
            button_face: c(0x000000),
        };
        let os = SystemSettings { high_contrast: true, system_colors: Some(colors), ..base };
        assert!(os.is_dark(light));
        let t = Tokens::themed(light, &os, "teal");
        assert_eq!((t.text, t.border, t.accent, t.on_accent), (colors.window_text, colors.window_text, colors.highlight, colors.highlight_text));
    }

    /// Reduced motion is a flag of the tokens that the renderer reads (no
    /// caret blink, no transitions).
    #[test]
    fn reduced_motion_sets_the_flag() {
        let base = SystemSettings::default();
        assert!(!Tokens::themed(WindowAppearance::Light, &base, "teal").reduced_motion);
        let reduced = SystemSettings { reduced_motion: true, ..base };
        assert!(Tokens::themed(WindowAppearance::Light, &reduced, "teal").reduced_motion);
        let both = SystemSettings { high_contrast: true, ..reduced };
        assert!(Tokens::themed(WindowAppearance::Dark, &both, "teal").reduced_motion);
    }
}
