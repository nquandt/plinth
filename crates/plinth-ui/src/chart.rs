//! Path-based geometry for `<Chart kind="line">` and `<Chart kind="pie">`
//! (SPEC.md §6.3, UI API 1.5). The pure helpers (scales, ticks, wedge
//! angles) are unit-tested; the painters draw them with gpui's `canvas`,
//! `PathBuilder` and `paint_path`. Colors always come from the runtime
//! theme (`Tokens::chart_palette`); apps never pick them.

use crate::theme::{Tokens, with_alpha};
use gpui::{AnyElement, Bounds, Hsla, PathBuilder, Pixels, Point, canvas, div, fill, point, prelude::*, px, size};
use std::f32::consts::{FRAC_PI_2, TAU};

/// One label/value pair.
pub(crate) type ChartPoints = Vec<(String, f64)>;

/// The height of one axis label row; the plot is inset by half of it at
/// the top and bottom so each gridline lines up with its tick label.
const LABEL_H: f32 = 16.;
const PAD: f32 = LABEL_H / 2.;

/// A y-axis scale: `lo..=hi` in steps of `step`, always including zero.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scale {
    pub lo: f64,
    pub hi: f64,
    pub step: f64,
}

impl Scale {
    /// A "nice" scale (steps of 1, 2, 2.5 or 5 times a power of ten, about
    /// four intervals) that covers every value and zero.
    pub fn for_values(values: impl IntoIterator<Item = f64>) -> Scale {
        let (mut min, mut max) = (0.0_f64, 0.0_f64);
        for v in values.into_iter().filter(|v| v.is_finite()) {
            min = min.min(v);
            max = max.max(v);
        }
        if max - min < 1e-12 {
            return Scale { lo: 0.0, hi: 1.0, step: 0.25 };
        }
        let raw = (max - min) / 4.0;
        let mag = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw * (1.0 - 1e-9)).unwrap_or(10.0 * mag);
        let lo = (min / step).floor() * step;
        let hi = (max / step).ceil() * step;
        Scale { lo, hi, step }
    }

    /// The tick values from `hi` down to `lo`.
    pub fn ticks_desc(&self) -> Vec<f64> {
        let n = ((self.hi - self.lo) / self.step).round() as i64;
        (0..=n).map(|i| self.hi - i as f64 * self.step).collect()
    }

    /// Where `v` falls between `lo` (0.0) and `hi` (1.0).
    pub fn frac(&self, v: f64) -> f32 {
        (((v - self.lo) / (self.hi - self.lo)) as f32).clamp(0.0, 1.0)
    }
}

/// A compact tick label: `1.5k`, `2M`, `12`, `0.25`.
pub(crate) fn format_tick(v: f64) -> String {
    let trim = |s: String| {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
    };
    let a = v.abs();
    let s = if a >= 1e6 {
        format!("{}M", trim(format!("{:.1}", v / 1e6)))
    } else if a >= 1e3 {
        format!("{}k", trim(format!("{:.1}", v / 1e3)))
    } else {
        trim(format!("{v:.2}"))
    };
    if s == "-0" { "0".into() } else { s }
}

/// The x of point `i` of `n` as a fraction of the plot width: the center
/// of the i-th of `n` equal columns (so the x labels, laid out as equal
/// flex columns, sit right under their points).
pub(crate) fn x_frac(i: usize, n: usize) -> f32 {
    (i as f32 + 0.5) / n.max(1) as f32
}

/// The wedges of a pie as `(index, start, end)` angles in radians,
/// clockwise from 12 o'clock. Negative values count by magnitude; zero
/// values get no wedge. An all-zero pie returns nothing.
pub(crate) fn pie_wedges(values: &[f64]) -> Vec<(usize, f32, f32)> {
    let total: f64 = values.iter().map(|v| if v.is_finite() { v.abs() } else { 0.0 }).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut start = -FRAC_PI_2;
    let mut out = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let v = if v.is_finite() { v.abs() } else { 0.0 };
        if v <= 0.0 {
            continue;
        }
        let sweep = (v / total) as f32 * TAU;
        out.push((i, start, start + sweep));
        start += sweep;
    }
    out
}

/// Each value's share of the whole as a whole percent (for the legend).
pub(crate) fn pie_percents(values: &[f64]) -> Vec<u32> {
    let total: f64 = values.iter().map(|v| if v.is_finite() { v.abs() } else { 0.0 }).sum();
    values.iter().map(|v| if total > 0.0 && v.is_finite() { (v.abs() / total * 100.0).round() as u32 } else { 0 }).collect()
}

fn on_circle(c: Point<Pixels>, r: f32, a: f32) -> Point<Pixels> {
    point(c.x + px(r * a.cos()), c.y + px(r * a.sin()))
}

/// Draws a line chart: a y axis with "nice" tick labels and light
/// gridlines, one stroked polyline per series in the theme's chart
/// palette, point markers when the points are not too dense, and the x
/// labels under their points (thinned to at most `max_x_labels`).
pub(crate) fn render_line(series: &[(String, ChartPoints)], t: &Tokens, height: f32, max_x_labels: usize) -> AnyElement {
    let palette = t.chart_palette();
    let scale = Scale::for_values(series.iter().flat_map(|(_, pts)| pts.iter().map(|(_, v)| *v)));
    let labels: Vec<String> = series.iter().map(|(_, p)| p).max_by_key(|p| p.len()).map(|p| p.iter().map(|(l, _)| l.clone()).collect()).unwrap_or_default();
    let n = labels.len().max(1);
    let every = n.div_ceil(max_x_labels.max(1));

    let ticks = scale.ticks_desc();
    let y_axis = div()
        .flex()
        .flex_col()
        .h_full()
        .child(
            div().flex().flex_col().justify_between().flex_1().children(ticks.iter().map(|v| {
                div().h(px(LABEL_H)).flex().items_center().justify_end().text_xs().text_color(t.text_muted).child(format_tick(*v))
            })),
        )
        // Leaves room for the x label row so the plot and axis line up.
        .child(div().h(px(LABEL_H)).flex_none());

    let lines: Vec<(Hsla, Vec<f64>)> =
        series.iter().enumerate().map(|(si, (_, pts))| (palette[si % palette.len()], pts.iter().map(|(_, v)| *v).collect())).collect();
    let grid = with_alpha(t.border, 0.6);
    let axis = t.border;
    let ring = t.surface;
    let tick_count = ticks.len();
    let plot = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let left = bounds.origin.x;
            let w = f32::from(bounds.size.width);
            let top = f32::from(bounds.origin.y) + PAD;
            let h = (f32::from(bounds.size.height) - 2. * PAD).max(1.);
            let y_of = |v: f64| px(top + (1. - scale.frac(v)) * h);
            for i in 0..tick_count {
                let v = scale.hi - i as f64 * scale.step;
                let color = if v.abs() < scale.step * 1e-6 { axis } else { grid };
                window.paint_quad(fill(Bounds::new(point(left, y_of(v)), size(bounds.size.width, px(1.))), color));
            }
            let markers = n as f32 <= w / 14.;
            for (color, values) in &lines {
                let pts: Vec<Point<Pixels>> =
                    values.iter().enumerate().map(|(i, v)| point(left + px(x_frac(i, n) * w), y_of(*v))).collect();
                if pts.len() > 1 {
                    let mut path = PathBuilder::stroke(px(2.5));
                    path.move_to(pts[0]);
                    for p in &pts[1..] {
                        path.line_to(*p);
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, *color);
                    }
                }
                if markers || pts.len() == 1 {
                    for p in &pts {
                        let dot = |r: f32| Bounds::new(point(p.x - px(r), p.y - px(r)), size(px(2. * r), px(2. * r)));
                        window.paint_quad(fill(dot(5.), ring).corner_radii(px(5.)));
                        window.paint_quad(fill(dot(3.5), *color).corner_radii(px(3.5)));
                    }
                }
            }
        },
    )
    .flex_1()
    .w_full();

    let x_labels = div().flex().h(px(LABEL_H)).flex_none().w_full().children(labels.iter().enumerate().map(|(i, l)| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .justify_center()
            .overflow_hidden()
            .text_xs()
            .text_color(t.text_muted)
            .when(i % every == 0, |d| d.child(l.clone()))
    }));

    div()
        .flex()
        .gap_2()
        .h(px(height))
        .w_full()
        .child(y_axis)
        .child(div().flex().flex_col().flex_1().h_full().child(plot).child(x_labels))
        .into_any_element()
}

/// Draws a pie: filled wedges per value in the theme's chart palette,
/// separated by thin lines in the surface color, with a legend (swatch,
/// label, share) beside it, or below it when `stacked` (compact width).
pub(crate) fn render_pie(points: &ChartPoints, t: &Tokens, diameter: f32, stacked: bool) -> AnyElement {
    let palette = t.chart_palette();
    let values: Vec<f64> = points.iter().map(|(_, v)| *v).collect();
    let wedges = pie_wedges(&values);
    let colors: Vec<Hsla> = (0..values.len()).map(|i| palette[i % palette.len()]).collect();
    let neutral = t.track;
    let gap = t.surface;
    let pie = canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, _| {
            let d = f32::from(bounds.size.width).min(f32::from(bounds.size.height));
            let r = d / 2. - 1.;
            let c = bounds.center();
            // Arcs as fine polygons: robust for any sweep (a full circle
            // included) and smooth at these sizes.
            let wedge_path = |a0: f32, a1: f32| {
                let steps = (((a1 - a0) / TAU * 180.).ceil() as usize).max(2);
                let mut path = PathBuilder::fill();
                path.move_to(c);
                for s in 0..=steps {
                    path.line_to(on_circle(c, r, a0 + (a1 - a0) * s as f32 / steps as f32));
                }
                path.close();
                path.build().ok()
            };
            if wedges.is_empty() {
                if let Some(p) = wedge_path(0., TAU) {
                    window.paint_path(p, neutral);
                }
                return;
            }
            for (i, a0, a1) in &wedges {
                if let Some(p) = wedge_path(*a0, *a1) {
                    window.paint_path(p, colors[*i]);
                }
            }
            if wedges.len() > 1 {
                for (_, a0, _) in &wedges {
                    let mut line = PathBuilder::stroke(px(2.));
                    line.move_to(c);
                    line.line_to(on_circle(c, r + 1., *a0));
                    if let Ok(p) = line.build() {
                        window.paint_path(p, gap);
                    }
                }
            }
        },
    )
    .size_full();

    let percents = pie_percents(&values);
    let entries = points.iter().enumerate().map(|(i, (label, _))| {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(px(10.)).h(px(10.)).flex_none().rounded_sm().bg(palette[i % palette.len()]))
            .child(div().text_sm().text_color(t.text).child(label.clone()))
            .child(div().text_sm().text_color(t.text_muted).child(format!("{}%", percents[i])))
    });
    let legend = if stacked {
        div().flex().flex_wrap().gap_x_3().gap_y_1().children(entries)
    } else {
        div().flex().flex_col().gap_1().children(entries)
    };
    let pie = div().w(px(diameter)).h(px(diameter)).flex_none().child(pie);
    if stacked {
        div().flex().flex_col().items_center().gap_3().child(pie).child(legend).into_any_element()
    } else {
        div().flex().items_center().gap_6().child(pie).child(legend).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_includes_zero_and_is_nice() {
        let s = Scale::for_values([120.0, 950.0, 400.0]);
        assert_eq!(s, Scale { lo: 0.0, hi: 1000.0, step: 250.0 });
        assert_eq!(s.ticks_desc(), vec![1000.0, 750.0, 500.0, 250.0, 0.0]);
        let neg = Scale::for_values([-30.0, 70.0]);
        assert!(neg.lo <= -30.0 && neg.hi >= 70.0 && neg.ticks_desc().contains(&0.0), "{neg:?}");
        assert_eq!(Scale::for_values([]), Scale { lo: 0.0, hi: 1.0, step: 0.25 });
        assert_eq!(Scale::for_values([f64::NAN, 0.0]).hi, 1.0);
    }

    #[test]
    fn scale_maps_values() {
        let s = Scale { lo: 0.0, hi: 100.0, step: 25.0 };
        assert_eq!(s.frac(50.0), 0.5);
        assert_eq!(s.frac(500.0), 1.0);
        assert_eq!(s.frac(-1.0), 0.0);
    }

    #[test]
    fn ticks_format_compactly() {
        assert_eq!(format_tick(0.0), "0");
        assert_eq!(format_tick(250.0), "250");
        assert_eq!(format_tick(2.5), "2.5");
        assert_eq!(format_tick(1500.0), "1.5k");
        assert_eq!(format_tick(2000.0), "2k");
        assert_eq!(format_tick(-3_000_000.0), "-3M");
    }

    #[test]
    fn x_positions_center_in_columns() {
        assert_eq!(x_frac(0, 1), 0.5);
        assert_eq!(x_frac(0, 4), 0.125);
        assert_eq!(x_frac(3, 4), 0.875);
    }

    #[test]
    fn pie_wedges_cover_the_circle() {
        let w = pie_wedges(&[1.0, 0.0, 3.0, -4.0]);
        assert_eq!(w.iter().map(|(i, _, _)| *i).collect::<Vec<_>>(), vec![0, 2, 3]);
        assert!((w[0].1 + FRAC_PI_2).abs() < 1e-6);
        assert!((w[2].2 - (TAU - FRAC_PI_2)).abs() < 1e-5);
        assert!(((w[0].2 - w[0].1) - TAU / 8.).abs() < 1e-5);
        assert!(pie_wedges(&[0.0, 0.0]).is_empty());
        assert_eq!(pie_percents(&[1.0, 3.0]), vec![25, 75]);
        assert_eq!(pie_percents(&[0.0]), vec![0]);
    }
}
