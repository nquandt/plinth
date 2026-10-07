//! `<Canvas>` (UI API 1.10, docs/UI-ADVANCED.md phase U4): shapes in a view
//! space of `view-width` x `view-height` that scales to the element. The
//! element keeps the aspect ratio of the view space, so a circle stays
//! round. Every shape is painted on a gpui canvas, texts too (shaped at the
//! scaled size, so text scales with the drawing as on the web). Colors are
//! theme tokens. The web host draws the same shapes as inline SVG
//! (`web/dom-renderer.js` `canvasView`).

use crate::primitives;
use crate::theme::Tokens;
use gpui::{
    BorderStyle, Bounds, Hsla, PathBuilder, Pixels, SharedString, TextAlign, TextRun, canvas, div, fill, outline, point, prelude::*, px, size,
};
use plinth_protocol::color;

/// One shape, in view units.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect { x: f32, y: f32, w: f32, h: f32, color: String },
    Circle { cx: f32, cy: f32, r: f32, color: String },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: String, width: f32 },
    Text { x: f32, y: f32, color: String, size: f32, text: String },
    /// UI API 1.13: the outline of a rectangle, `width` in view units.
    StrokeRect { x: f32, y: f32, w: f32, h: f32, color: String, width: f32 },
    /// UI API 1.13: the outline of a circle.
    StrokeCircle { cx: f32, cy: f32, r: f32, color: String, width: f32 },
}

/// The shapes of a `shapes` prop. A malformed shape is skipped.
pub fn parse(shapes: &str) -> Vec<Shape> {
    shapes.split('\u{1e}').filter(|s| !s.is_empty()).filter_map(parse_one).collect()
}

fn parse_one(s: &str) -> Option<Shape> {
    // The text of a "t" shape is the last field and may hold the separator.
    let limit = if s.starts_with("t\u{1f}") { 6 } else { 7 };
    let f: Vec<&str> = s.splitn(limit, '\u{1f}').collect();
    let n = |i: usize| f.get(i).and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite());
    let c = |i: usize| f.get(i).map(|v| (*v).to_owned());
    Some(match *f.first()? {
        "r" => Shape::Rect { x: n(1)?, y: n(2)?, w: n(3)?, h: n(4)?, color: c(5)? },
        "c" => Shape::Circle { cx: n(1)?, cy: n(2)?, r: n(3)?, color: c(4)? },
        "l" => Shape::Line { x1: n(1)?, y1: n(2)?, x2: n(3)?, y2: n(4)?, color: c(5)?, width: n(6).unwrap_or(1.0) },
        "t" => Shape::Text { x: n(1)?, y: n(2)?, color: c(3)?, size: n(4).unwrap_or(12.0), text: c(5).unwrap_or_default() },
        "R" => Shape::StrokeRect { x: n(1)?, y: n(2)?, w: n(3)?, h: n(4)?, color: c(5)?, width: n(6).unwrap_or(1.0) },
        "C" => Shape::StrokeCircle { cx: n(1)?, cy: n(2)?, r: n(3)?, color: c(4)?, width: n(5).unwrap_or(1.0) },
        _ => return None,
    })
}

/// The theme color of a token name (`"accent"`, `"text.muted"`, ...).
pub fn token_color(t: &Tokens, name: &str) -> Option<Hsla> {
    let id = match name {
        "background" => color::BACKGROUND,
        "surface" => color::SURFACE,
        "surface.alt" => color::SURFACE_ALT,
        "accent" => color::ACCENT,
        "danger" => color::DANGER,
        "success" => color::SUCCESS,
        "text" => color::TEXT,
        "text.muted" => color::TEXT_MUTED,
        "on.accent" => color::ON_ACCENT,
        "border" => color::BORDER,
        "hover" => color::HOVER,
        "selected" => color::SELECTED,
        _ => return None,
    };
    primitives::token(t, id)
}

/// The canvas element: `vw` x `vh` view units, `shapes`, the accessible `label`.
pub fn render(id: u64, label: String, vw: f32, vh: f32, shapes: Vec<Shape>, t: &Tokens) -> gpui::Stateful<gpui::Div> {
    let (vw, vh) = (vw.max(1.0), vh.max(1.0));
    let tokens = *t;
    let painted = shapes.clone();
    let surface = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let k = f32::from(bounds.size.width) / vw;
            let at = |x: f32, y: f32| point(bounds.origin.x + px(x * k), bounds.origin.y + px(y * k));
            for s in &painted {
                match s {
                    Shape::Rect { x, y, w, h, color } => {
                        if let Some(c) = token_color(&tokens, color) {
                            window.paint_quad(fill(Bounds::new(at(*x, *y), size(px(w * k), px(h * k))), c));
                        }
                    }
                    Shape::Circle { cx, cy, r, color } => {
                        if let Some(c) = token_color(&tokens, color) {
                            let d = px(2.0 * r * k);
                            window.paint_quad(fill(Bounds::new(at(cx - r, cy - r), size(d, d)), c).corner_radii(px(r * k)));
                        }
                    }
                    // An outline is a border inside the shape's bounds, as an SVG stroke
                    // is centered on the edge: the bounds grow by half the line width.
                    Shape::StrokeRect { x, y, w, h, color, width } => {
                        if let Some(c) = token_color(&tokens, color) {
                            let lw = width.max(0.0);
                            let b = Bounds::new(at(x - lw / 2.0, y - lw / 2.0), size(px((w + lw) * k), px((h + lw) * k)));
                            window.paint_quad(outline(b, c, BorderStyle::Solid).border_widths(px((lw * k).max(0.5))));
                        }
                    }
                    Shape::StrokeCircle { cx, cy, r, color, width } => {
                        if let Some(c) = token_color(&tokens, color) {
                            let lw = width.max(0.0);
                            let outer = r + lw / 2.0;
                            let d = px(2.0 * outer * k);
                            let b = Bounds::new(at(cx - outer, cy - outer), size(d, d));
                            window.paint_quad(outline(b, c, BorderStyle::Solid).border_widths(px((lw * k).max(0.5))).corner_radii(px(outer * k)));
                        }
                    }
                    Shape::Line { x1, y1, x2, y2, color, width } => {
                        if let Some(c) = token_color(&tokens, color) {
                            let mut path = PathBuilder::stroke(px((width * k).max(0.5)));
                            path.move_to(at(*x1, *y1));
                            path.line_to(at(*x2, *y2));
                            if let Ok(path) = path.build() {
                                window.paint_path(path, c);
                            }
                        }
                    }
                    Shape::Text { x, y, color, size: text_size, text } => {
                        let (Some(c), false) = (token_color(&tokens, color), text.is_empty()) else { continue };
                        let font_size = px((text_size * k).max(1.0));
                        let line: SharedString = text.replace('\n', " ").into();
                        let run = TextRun {
                            len: line.len(),
                            font: window.text_style().font(),
                            color: c,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                            letter_spacing: None,
                        };
                        let shaped = window.text_system().shape_line(line, font_size, &[run]);
                        // (x, y) is the left baseline: the line box starts about one ascent higher.
                        let origin = at(*x, *y - text_size * 0.8);
                        let _ = shaped.paint(origin, font_size * 1.2, TextAlign::Left, None, window, cx);
                    }
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full();
    div()
        .id(("canvas", id))
        .role(gpui::accesskit::Role::Image)
        .aria_label(label)
        .relative()
        .w_full()
        .aspect_ratio(vw / vh)
        .overflow_hidden()
        .child(surface)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind_and_skips_bad_shapes() {
        let s = "r\u{1f}1\u{1f}2\u{1f}3\u{1f}4\u{1f}accent\u{1e}c\u{1f}5\u{1f}6\u{1f}7\u{1f}text\u{1e}x\u{1f}1\u{1e}\
                 l\u{1f}0\u{1f}0\u{1f}10\u{1f}10\u{1f}border\u{1f}2\u{1e}t\u{1f}1\u{1f}20\u{1f}text.muted\u{1f}14\u{1f}Hi\u{1f}there\u{1e}r\u{1f}a";
        let shapes = parse(s);
        assert_eq!(shapes.len(), 4, "{shapes:?}");
        assert_eq!(shapes[0], Shape::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0, color: "accent".into() });
        assert_eq!(shapes[1], Shape::Circle { cx: 5.0, cy: 6.0, r: 7.0, color: "text".into() });
        assert_eq!(shapes[2], Shape::Line { x1: 0.0, y1: 0.0, x2: 10.0, y2: 10.0, color: "border".into(), width: 2.0 });
        // The text is the last field and may hold the field separator.
        assert_eq!(shapes[3], Shape::Text { x: 1.0, y: 20.0, color: "text.muted".into(), size: 14.0, text: "Hi\u{1f}there".into() });
    }

    #[test]
    fn parses_outlines() {
        let s = "R\u{1f}1\u{1f}2\u{1f}3\u{1f}4\u{1f}accent\u{1f}2\u{1e}C\u{1f}5\u{1f}6\u{1f}7\u{1f}text\u{1e}C\u{1f}5\u{1f}6";
        assert_eq!(
            parse(s),
            [
                Shape::StrokeRect { x: 1.0, y: 2.0, w: 3.0, h: 4.0, color: "accent".into(), width: 2.0 },
                Shape::StrokeCircle { cx: 5.0, cy: 6.0, r: 7.0, color: "text".into(), width: 1.0 },
            ]
        );
    }
}
