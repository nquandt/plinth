//! Level 2 styled primitives (UI API 1.6, docs/UI-ADVANCED.md phase U1):
//! the style props of `Box`, `Pressable`, `Scroll` and `Span` as gpui
//! styles. Spaces and sizes are spacing units (one unit is 4 px, the same
//! on every host); colors are theme tokens. The web host maps the same
//! props in `web/dom-renderer.js`.

use crate::theme::Tokens;
use crate::tree::Node;
use gpui::{FontWeight, Hsla, Length, Pixels, Styled, px, relative};
use plinth_protocol::{Value, axis, color, cross_align, fraction, justify, prop, radius, text_size, weight};

/// One spacing unit, in pixels.
pub const UNIT: f32 = 4.0;

/// The most spacing units that a prop can give (larger values are clamped).
pub const MAX_UNITS: i64 = 512;

/// `n` spacing units as pixels.
pub fn units(n: i64) -> Pixels {
    px(n.clamp(0, MAX_UNITS) as f32 * UNIT)
}

fn int(node: &Node, id: u16) -> Option<i64> {
    node.prop(id).and_then(Value::as_int).map(i64::from)
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

/// A size prop: spacing units if the int prop is set, else the fraction.
pub fn size(node: &Node, units_prop: u16, fraction_prop: u16) -> Option<Length> {
    if let Some(n) = int(node, units_prop) {
        return Some(units(n).into());
    }
    let share = match node.enum_prop(fraction_prop) {
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
pub fn is_row(node: &Node) -> bool {
    node.enum_prop(prop::AXIS) == axis::ROW
}

/// The `grow` prop of any primitive: its share of the free space in a parent box.
pub fn grow<S: Styled>(d: S, node: &Node) -> S {
    match int(node, prop::GROW) {
        Some(g) if g > 0 => d.flex_grow(g.min(100) as f32).flex_basis(px(0.)).min_w_0(),
        _ => d,
    }
}

/// The box props: direction, wrap, gap, padding, alignment, grow, sizes, background, border, radius.
pub fn box_style<S: Styled>(d: S, node: &Node, t: &Tokens) -> S {
    let mut d = d.flex();
    d = if is_row(node) { d.flex_row() } else { d.flex_col() };
    if node.bool_prop(prop::WRAP) {
        d = d.flex_wrap();
    }
    if let Some(g) = int(node, prop::GAP) {
        d = d.gap(units(g));
    }
    if let Some(p) = int(node, prop::PADDING) {
        d = d.p(units(p));
    }
    if let Some(p) = int(node, prop::PADDING_X) {
        d = d.px(units(p));
    }
    if let Some(p) = int(node, prop::PADDING_Y) {
        d = d.py(units(p));
    }
    d = match node.enum_prop(prop::CROSS_ALIGN) {
        cross_align::START => d.items_start(),
        cross_align::CENTER => d.items_center(),
        cross_align::END => d.items_end(),
        _ => d.items_stretch(),
    };
    d = match node.enum_prop(prop::JUSTIFY) {
        justify::CENTER => d.justify_center(),
        justify::END => d.justify_end(),
        justify::BETWEEN => d.justify_between(),
        _ => d.justify_start(),
    };
    d = grow(d, node);
    // A size in spacing units is fixed: the element does not shrink below it.
    if int(node, prop::WIDTH).is_some() || int(node, prop::HEIGHT).is_some() {
        d = d.flex_shrink_0();
    }
    if let Some(w) = size(node, prop::WIDTH, prop::WIDTH_FRACTION) {
        d = d.w(w);
    }
    if let Some(h) = size(node, prop::HEIGHT, prop::HEIGHT_FRACTION) {
        d = d.h(h);
    }
    if let Some(w) = size(node, prop::MAX_WIDTH, prop::MAX_WIDTH_FRACTION) {
        d = d.max_w(w);
    }
    if let Some(h) = size(node, prop::MAX_HEIGHT, prop::MAX_HEIGHT_FRACTION) {
        d = d.max_h(h);
    }
    if let Some(c) = token(t, node.enum_prop(prop::BG)) {
        d = d.bg(c);
    }
    if let Some(c) = token(t, node.enum_prop(prop::BORDER)) {
        d = d.border_1().border_color(c);
    }
    match node.enum_prop(prop::RADIUS) {
        radius::SM => d.rounded(px(4.)),
        radius::MD => d.rounded(px(8.)),
        radius::LG => d.rounded(px(12.)),
        radius::FULL => d.rounded_full(),
        _ => d,
    }
}

/// The span props: size, weight, italic, mono, color, and the line limit
/// (`align` and `grow` are applied by the caller).
pub fn span_style<S: Styled>(d: S, node: &Node, t: &Tokens) -> S {
    let mut d = match node.enum_prop(prop::TEXT_SIZE) {
        text_size::XS => d.text_xs(),
        text_size::MD => d.text_base(),
        text_size::LG => d.text_lg(),
        text_size::XL => d.text_xl(),
        text_size::XXL => d.text_2xl(),
        _ => d.text_sm(),
    };
    d = d.font_weight(match node.enum_prop(prop::WEIGHT) {
        weight::MEDIUM => FontWeight::MEDIUM,
        weight::SEMIBOLD => FontWeight::SEMIBOLD,
        weight::BOLD => FontWeight::BOLD,
        _ => FontWeight::NORMAL,
    });
    if node.bool_prop(prop::ITALIC) {
        d = d.italic();
    }
    if node.bool_prop(prop::MONO) {
        d = d.font_family("monospace");
    }
    d = d.text_color(token(t, node.enum_prop(prop::FG)).unwrap_or(t.text));
    match int(node, prop::LINES) {
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
}
