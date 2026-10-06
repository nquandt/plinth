//! Runtime-owned theme tokens (SPEC.md §6.5) and width classes (SPEC.md §6.1).

use gpui::{Hsla, Pixels, WindowAppearance, px, rgb, rgb_to_hsla};

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

fn with_alpha(color: Hsla, alpha: f32) -> Hsla {
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
            }
        }
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
        _ => "•",
    }
}
