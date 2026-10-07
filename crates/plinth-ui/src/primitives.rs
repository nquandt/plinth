//! Level 2 styled primitives (docs/UI-ADVANCED.md): the style props of
//! `Box`, `Pressable`, `Scroll` and `Span` as gpui styles. Spaces and sizes
//! are spacing units (one unit is 4 px, the same on every host); colors are
//! theme tokens. The web host maps the same props in `web/dom-renderer.js`.
//!
//! UI API 1.6 (phase U1) gives the style props. UI API 1.7 (phase U2) adds
//! partial styles: `hover`, `active`, `focus` and the width classes
//! `compact`, `regular`, `wide`. Each is a string of `"<prop id>:<int>"`
//! pairs (made by the compiler); the host applies it over the props.

use crate::theme::{Tokens, WidthClass};
use crate::tree::Node;
use gpui::{FontWeight, Hsla, Length, Pixels, Styled, px, relative};
use plinth_protocol::{Value, axis, color, cross_align, fraction, justify, position, prop, radius, text_size, weight};

/// One spacing unit, in pixels.
pub const UNIT: f32 = 4.0;

/// The most spacing units that a prop can give (larger values are clamped).
pub const MAX_UNITS: i64 = 512;

/// `n` spacing units as pixels.
pub fn units(n: i64) -> Pixels {
    px(n.clamp(0, MAX_UNITS) as f32 * UNIT)
}

/// `n` spacing units as pixels; a size or an inset can be fractional
/// (UI API 1.11), so a moving box does not jump by whole units.
pub fn units_f(n: f64) -> Pixels {
    let n = if n.is_finite() { n.clamp(0.0, MAX_UNITS as f64) } else { 0.0 };
    px(n as f32 * UNIT)
}

/// The size props, each with its fraction prop. A style that sets one of a
/// pair replaces the other.
const SIZE_PAIRS: [(u16, u16); 4] = [
    (prop::WIDTH, prop::WIDTH_FRACTION),
    (prop::HEIGHT, prop::HEIGHT_FRACTION),
    (prop::MAX_WIDTH, prop::MAX_WIDTH_FRACTION),
    (prop::MAX_HEIGHT, prop::MAX_HEIGHT_FRACTION),
];

/// The style props of one element as `(prop id, value)`: enums as their
/// value, booleans as 0 or 1. Sizes and insets can be fractional.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    values: Vec<(u16, f64)>,
}

impl Style {
    /// The style props of `node` (its other props are left out).
    pub fn of(node: &Node) -> Self {
        let values = node
            .props
            .iter()
            .filter(|(id, _)| {
                *id == prop::AXIS || *id == prop::ALIGN || (prop::WRAP..=prop::LINES).contains(id) || (prop::POSITION..=prop::BOTTOM).contains(id)
            })
            .filter_map(|(id, v)| {
                let n = match v {
                    Value::Int(i) => f64::from(*i),
                    Value::Number(n) => *n,
                    Value::Enum(e) => f64::from(*e),
                    Value::Bool(b) => f64::from(u8::from(*b)),
                    _ => return None,
                };
                Some((*id, n))
            })
            .collect();
        Self { values }
    }

    /// The style of `node` in the width class `class`: the props, then the
    /// partial style of that class over them.
    pub fn for_class(node: &Node, class: WidthClass) -> Self {
        let mut style = Self::of(node);
        let id = match class {
            WidthClass::Compact => prop::COMPACT,
            WidthClass::Regular => prop::REGULAR,
            WidthClass::Wide => prop::WIDE,
        };
        if let Some(text) = node.str_prop(id) {
            style.overlay(&Self::parse(text));
        }
        style
    }

    /// A partial style: `"56:11,42:4"` (`"73:1.5"` for a fractional size).
    /// Malformed pairs are skipped.
    pub fn parse(text: &str) -> Self {
        let values = text
            .split(',')
            .filter_map(|pair| {
                let (k, v) = pair.split_once(':')?;
                let v: f64 = v.trim().parse().ok()?;
                v.is_finite().then_some((k.trim().parse().ok()?, v))
            })
            .collect();
        Self { values }
    }

    /// Sets each value of `top` over this style.
    pub fn overlay(&mut self, top: &Style) {
        for &(id, v) in &top.values {
            for (a, b) in SIZE_PAIRS {
                if id == a || id == b {
                    let other = if id == a { b } else { a };
                    self.values.retain(|(k, _)| *k != other);
                }
            }
            match self.values.iter_mut().find(|(k, _)| *k == id) {
                Some(slot) => slot.1 = v,
                None => self.values.push((id, v)),
            }
        }
    }

    /// The value of `id` as a whole number (a fraction is cut off).
    pub fn get(&self, id: u16) -> Option<i64> {
        self.get_f(id).map(|v| v as i64)
    }

    pub fn get_f(&self, id: u16) -> Option<f64> {
        self.values.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
    }

    pub fn enum_(&self, id: u16) -> u16 {
        self.get(id).and_then(|v| u16::try_from(v).ok()).unwrap_or(0)
    }

    pub fn flag(&self, id: u16) -> bool {
        self.get(id).is_some_and(|v| v != 0)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// The Plinth name of a gpui key (UI API 1.9): the same names as the web
/// host (`dom-renderer.js` `keyName`). `None` for a key that apps do not get.
pub fn key_name(key: &str) -> Option<String> {
    let name = match key {
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        "enter" => "Enter",
        "escape" => "Escape",
        "space" => "Space",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        k if k.chars().count() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => return Some(k.to_ascii_lowercase()),
        _ => return None,
    };
    Some(name.to_owned())
}

/// The partial style that `node` gives in its string prop `id` (`hover`, ...).
pub fn partial(node: &Node, id: u16) -> Option<Style> {
    node.str_prop(id).map(Style::parse).filter(|s| !s.is_empty())
}

/// A color token, or `None` for `none` and for an unknown value.
pub fn token(t: &Tokens, value: u16) -> Option<Hsla> {
    Some(match value {
        color::BACKGROUND => t.background,
        color::SURFACE => t.surface,
        color::SURFACE_ALT => t.surface_alt,
        color::ACCENT => t.accent,
        color::DANGER => t.danger,
        color::SUCCESS => t.success,
        color::TEXT => t.text,
        color::TEXT_MUTED => t.text_muted,
        color::ON_ACCENT => t.on_accent,
        color::BORDER => t.border,
        color::HOVER => t.hover,
        color::SELECTED => t.selected,
        _ => return None,
    })
}

/// A size: spacing units if the int prop is set, else the fraction.
pub fn size(s: &Style, units_prop: u16, fraction_prop: u16) -> Option<Length> {
    if let Some(n) = s.get_f(units_prop) {
        return Some(units_f(n).into());
    }
    let share = match s.enum_(fraction_prop) {
        fraction::AUTO => return Some(Length::Auto),
        fraction::FULL => 1.0,
        fraction::HALF => 1.0 / 2.0,
        fraction::THIRD => 1.0 / 3.0,
        fraction::TWO_THIRDS => 2.0 / 3.0,
        fraction::QUARTER => 1.0 / 4.0,
        fraction::THREE_QUARTERS => 3.0 / 4.0,
        _ => return None,
    };
    Some(relative(share).into())
}

/// Whether the children of a box go in a row (the default is a column).
pub fn is_row(s: &Style) -> bool {
    s.enum_(prop::AXIS) == axis::ROW
}

/// The `grow` prop of any primitive: its share of the free space in a parent box.
pub fn grow<S: Styled>(d: S, s: &Style) -> S {
    match s.get(prop::GROW) {
        Some(g) if g > 0 => d.flex_grow(g.min(100) as f32).flex_basis(px(0.)).min_w_0(),
        _ => d,
    }
}

/// The box props. A `partial` style applies only the props that it sets;
/// a full style also applies the defaults (a column that stretches its
/// children, from the start).
pub fn box_style<S: Styled>(d: S, s: &Style, t: &Tokens, partial: bool) -> S {
    let has = |id: u16| s.get(id).is_some();
    let mut d = if partial { d } else { d.flex() };
    if !partial || has(prop::AXIS) {
        d = if is_row(s) { d.flex_row() } else { d.flex_col() };
    }
    if s.flag(prop::WRAP) {
        d = d.flex_wrap();
    }
    if let Some(g) = s.get(prop::GAP) {
        d = d.gap(units(g));
    }
    if let Some(p) = s.get(prop::PADDING) {
        d = d.p(units(p));
    }
    if let Some(p) = s.get(prop::PADDING_X) {
        d = d.px(units(p));
    }
    if let Some(p) = s.get(prop::PADDING_Y) {
        d = d.py(units(p));
    }
    if !partial || has(prop::CROSS_ALIGN) {
        d = match s.enum_(prop::CROSS_ALIGN) {
            cross_align::START => d.items_start(),
            cross_align::CENTER => d.items_center(),
            cross_align::END => d.items_end(),
            _ => d.items_stretch(),
        };
    }
    if !partial || has(prop::JUSTIFY) {
        d = match s.enum_(prop::JUSTIFY) {
            justify::CENTER => d.justify_center(),
            justify::END => d.justify_end(),
            justify::BETWEEN => d.justify_between(),
            _ => d.justify_start(),
        };
    }
    d = grow(d, s);
    // A size in spacing units is fixed: the element does not shrink below it.
    if has(prop::WIDTH) || has(prop::HEIGHT) {
        d = d.flex_shrink_0();
    }
    if let Some(w) = size(s, prop::WIDTH, prop::WIDTH_FRACTION) {
        d = d.w(w);
    }
    if let Some(h) = size(s, prop::HEIGHT, prop::HEIGHT_FRACTION) {
        d = d.h(h);
    }
    if let Some(w) = size(s, prop::MAX_WIDTH, prop::MAX_WIDTH_FRACTION) {
        d = d.max_w(w);
    }
    if let Some(h) = size(s, prop::MAX_HEIGHT, prop::MAX_HEIGHT_FRACTION) {
        d = d.max_h(h);
    }
    if let Some(c) = token(t, s.enum_(prop::BG)) {
        d = d.bg(c);
    }
    if let Some(c) = token(t, s.enum_(prop::BORDER)) {
        d = d.border_1().border_color(c);
    }
    d = match s.enum_(prop::RADIUS) {
        radius::SM => d.rounded(px(4.)),
        radius::MD => d.rounded(px(8.)),
        radius::LG => d.rounded(px(12.)),
        radius::FULL => d.rounded_full(),
        _ => d,
    };
    // UI API 1.9: an absolute box is placed in its parent by the insets.
    if !partial || has(prop::POSITION) {
        d = if s.enum_(prop::POSITION) == position::ABSOLUTE { d.absolute() } else { d.relative() };
    }
    if let Some(v) = s.get_f(prop::TOP) {
        d = d.top(units_f(v));
    }
    if let Some(v) = s.get_f(prop::LEFT) {
        d = d.left(units_f(v));
    }
    if let Some(v) = s.get_f(prop::RIGHT) {
        d = d.right(units_f(v));
    }
    if let Some(v) = s.get_f(prop::BOTTOM) {
        d = d.bottom(units_f(v));
    }
    d
}

/// The span props: size, weight, italic, mono, color, and the line limit
/// (`align` and `grow` are applied by the caller).
pub fn span_style<S: Styled>(d: S, s: &Style, t: &Tokens) -> S {
    let mut d = match s.enum_(prop::TEXT_SIZE) {
        text_size::XS => d.text_xs(),
        text_size::MD => d.text_base(),
        text_size::LG => d.text_lg(),
        text_size::XL => d.text_xl(),
        text_size::XXL => d.text_2xl(),
        _ => d.text_sm(),
    };
    d = d.font_weight(match s.enum_(prop::WEIGHT) {
        weight::MEDIUM => FontWeight::MEDIUM,
        weight::SEMIBOLD => FontWeight::SEMIBOLD,
        weight::BOLD => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    });
    if s.flag(prop::ITALIC) {
        d = d.italic();
    }
    if s.flag(prop::MONO) {
        d = d.font_family("monospace");
    }
    d = d.text_color(token(t, s.enum_(prop::FG)).unwrap_or(t.text));
    match s.get(prop::LINES) {
        Some(n) if n > 0 => d.line_clamp(n.min(1000) as usize),
        _ => d,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_four_pixels_and_clamped() {
        assert_eq!(units(3), px(12.));
        assert_eq!(units(-2), px(0.));
        assert_eq!(units(10_000), px(MAX_UNITS as f32 * UNIT));
    }

    #[test]
    fn key_names_match_the_web() {
        assert_eq!(key_name("up").as_deref(), Some("ArrowUp"));
        assert_eq!(key_name("space").as_deref(), Some("Space"));
        assert_eq!(key_name("W").as_deref(), Some("w"));
        assert_eq!(key_name("7").as_deref(), Some("7"));
        assert_eq!(key_name("f13"), None);
    }

    #[test]
    fn parse_skips_malformed_pairs() {
        let s = Style::parse("56:11, 42:4,x:1,7,41:-2");
        assert_eq!(s.get(prop::BG), Some(11));
        assert_eq!(s.get(prop::PADDING), Some(4));
        assert_eq!(s.get(prop::GAP), Some(-2));
        assert_eq!(s.get(prop::AXIS), None);
    }

    #[test]
    fn sizes_and_insets_can_be_fractional() {
        let s = Style::parse(&format!("{}:2.25,{}:10.5,{}:1e400", prop::LEFT, prop::WIDTH, prop::TOP));
        assert_eq!(s.get_f(prop::LEFT), Some(2.25));
        assert_eq!(s.get(prop::LEFT), Some(2), "a whole-number read cuts the fraction off");
        assert_eq!(s.get_f(prop::TOP), None, "an infinite value is skipped");
        assert_eq!(units_f(2.25), px(9.));
        assert_eq!(units_f(-1.5), px(0.));
        assert_eq!(units_f(f64::NAN), px(0.));
        assert_eq!(size(&s, prop::WIDTH, prop::WIDTH_FRACTION), Some(px(42.).into()));
    }

    #[test]
    fn overlay_replaces_values_and_the_other_size_form() {
        let mut base = Style::parse(&format!("{}:2,{}:3", prop::WIDTH_FRACTION, prop::GAP));
        base.overlay(&Style::parse(&format!("{}:40,{}:1", prop::WIDTH, prop::GAP)));
        assert_eq!(base.get(prop::WIDTH), Some(40));
        assert_eq!(base.get(prop::WIDTH_FRACTION), None, "units replace the fraction");
        assert_eq!(base.get(prop::GAP), Some(1));
    }
}
