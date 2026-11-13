//! `<canvas>`: shapes as child elements, in pixels from its top left,
//! painted with GPUI's `canvas` (the one way in to painting). And the
//! timing of `animate="..."`.
//!
//! ```heex
//! <canvas class="w-32 h-16">
//!   <rect x="0" y="0" width="10" height="20" rx="2" class="fill-primary" />
//!   <circle cx="5" cy="5" r="3" class="fill-danger stroke-border" />
//!   <path d="M 0 0 L 10 10 C 20 0 30 20 40 10 Z" class="stroke-primary" stroke-width="2" />
//!   <polyline points="0,0 10,10 20,5" class="stroke-muted" />
//!   <line x1="0" y1="0" x2="10" y2="10" class="stroke-primary" />
//! </canvas>
//! ```
//!
//! `fill-<color>` fills a shape and `stroke-<color>` outlines it, with
//! `stroke-width` (default 1).

use std::collections::BTreeMap;

use tauri_plugin_gpui::gpui::{self, Hsla, Pixels, Point, point, px};

use crate::style::{self, Theme};
use crate::view::Node;

#[derive(Debug, PartialEq, Clone)]
pub enum Segment {
    Move(f32, f32),
    Line(f32, f32),
    /// To `(x, y)` with control points `(x1, y1)` and `(x2, y2)`.
    Cubic(f32, f32, f32, f32, f32, f32),
    /// To `(x, y)` with control point `(x1, y1)`.
    Quad(f32, f32, f32, f32),
    Close,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Geometry {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    Path(Vec<Segment>),
}

#[derive(Debug, PartialEq, Clone)]
pub struct Shape {
    pub geometry: Geometry,
    pub fill: Option<Hsla>,
    pub stroke: Option<Hsla>,
    pub stroke_width: f32,
}

/// The shapes among a canvas's children; anything else is skipped.
pub fn shapes(children: &[Node], theme: &Theme) -> Vec<Shape> {
    children
        .iter()
        .filter_map(|child| match child {
            Node::Element { tag, attrs, .. } => shape(tag, attrs, theme),
            Node::Text(_) => None,
        })
        .collect()
}

fn shape(tag: &str, attrs: &BTreeMap<String, String>, theme: &Theme) -> Option<Shape> {
    let at = |key: &str| {
        attrs
            .get(key)
            .and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(0.)
    };
    let geometry = match tag {
        "rect" => Geometry::Rect {
            x: at("x"),
            y: at("y"),
            width: at("width"),
            height: at("height"),
            radius: at("rx"),
        },
        "circle" => Geometry::Circle {
            cx: at("cx"),
            cy: at("cy"),
            r: at("r"),
        },
        "line" => Geometry::Path(vec![
            Segment::Move(at("x1"), at("y1")),
            Segment::Line(at("x2"), at("y2")),
        ]),
        "polyline" | "polygon" => {
            let mut points = numbers(attrs.get("points").map(String::as_str).unwrap_or(""))
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .map(|(i, &[x, y])| match i {
                    0 => Segment::Move(x, y),
                    _ => Segment::Line(x, y),
                })
                .collect::<Vec<_>>();
            if tag == "polygon" {
                points.push(Segment::Close);
            }
            Geometry::Path(points)
        }
        "path" => Geometry::Path(path(attrs.get("d").map(String::as_str).unwrap_or(""))),
        _ => return None,
    };
    let classes = attrs.get("class").map(String::as_str).unwrap_or("");
    let color = |prefix: &str| {
        classes
            .split_whitespace()
            .find_map(|class| style::color(class.strip_prefix(prefix)?, theme))
    };
    let (fill, stroke) = (color("fill-"), color("stroke-"));
    // Like SVG, a line with no stroke class is drawn in the text colour's
    // absence: black. A closed shape with neither is filled black.
    let (fill, stroke) = match (&geometry, fill, stroke) {
        (Geometry::Path(_), None, None) if tag != "polygon" && tag != "path" => {
            (None, Some(gpui::black()))
        }
        (_, None, None) => (Some(gpui::black()), None),
        (_, fill, stroke) => (fill, stroke),
    };
    Some(Shape {
        geometry,
        fill,
        stroke,
        stroke_width: attrs
            .get("stroke-width")
            .and_then(|w| w.parse().ok())
            .unwrap_or(1.),
    })
}

fn numbers(text: &str) -> Vec<f32> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// SVG path data with absolute `M`, `L`, `H`, `V`, `C`, `Q` and `Z`.
/// Anything it doesn't know ends the path there.
pub fn path(d: &str) -> Vec<Segment> {
    // Commands as their own tokens: "M0 0L10,10" is "M 0 0 L 10 10".
    let spaced: String = d
        .chars()
        .flat_map(|c| match c {
            c if c.is_ascii_alphabetic() => vec![' ', c, ' '],
            ',' => vec![' '],
            c => vec![c],
        })
        .collect();
    let mut tokens = spaced.split_whitespace().peekable();
    let mut segments = Vec::new();
    let (mut x, mut y) = (0., 0.);
    while let Some(command) = tokens.next() {
        let mut take = |n: usize| -> Option<Vec<f32>> {
            (0..n).map(|_| tokens.next()?.parse::<f32>().ok()).collect()
        };
        let segment = match command {
            "M" => take(2).map(|p| Segment::Move(p[0], p[1])),
            "L" => take(2).map(|p| Segment::Line(p[0], p[1])),
            "H" => take(1).map(|p| Segment::Line(p[0], y)),
            "V" => take(1).map(|p| Segment::Line(x, p[0])),
            "C" => take(6).map(|p| Segment::Cubic(p[0], p[1], p[2], p[3], p[4], p[5])),
            "Q" => take(4).map(|p| Segment::Quad(p[0], p[1], p[2], p[3])),
            "Z" | "z" => Some(Segment::Close),
            _ => None,
        };
        let Some(segment) = segment else {
            break;
        };
        match segment {
            Segment::Move(px, py) | Segment::Line(px, py) => (x, y) = (px, py),
            Segment::Cubic(.., px, py) | Segment::Quad(.., px, py) => (x, y) = (px, py),
            Segment::Close => {}
        }
        segments.push(segment);
    }
    segments
}

/// Paints `shapes` with `origin` as their (0, 0).
pub fn paint(shapes: &[Shape], origin: Point<Pixels>, window: &mut gpui::Window) {
    let at = |x: f32, y: f32| origin + point(px(x), px(y));
    for shape in shapes {
        match &shape.geometry {
            Geometry::Rect {
                x,
                y,
                width,
                height,
                radius,
            } => quad(shape, at(*x, *y), *width, *height, *radius, window),
            Geometry::Circle { cx, cy, r } => {
                quad(shape, at(cx - r, cy - r), r * 2., r * 2., *r, window)
            }
            Geometry::Path(segments) => {
                let closed = segments.contains(&Segment::Close);
                if let (Some(fill), true) = (shape.fill, closed) {
                    let mut builder = gpui::PathBuilder::fill();
                    trace(&mut builder, segments, &at);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, fill);
                    }
                }
                if let Some(stroke) = shape.stroke {
                    let mut builder = gpui::PathBuilder::stroke(px(shape.stroke_width));
                    trace(&mut builder, segments, &at);
                    if let Ok(path) = builder.build() {
                        window.paint_path(path, stroke);
                    }
                }
            }
        }
    }
}

fn quad(
    shape: &Shape,
    origin: Point<Pixels>,
    width: f32,
    height: f32,
    radius: f32,
    window: &mut gpui::Window,
) {
    let bounds = gpui::Bounds::new(origin, gpui::size(px(width), px(height)));
    let fill = shape.fill.unwrap_or(gpui::transparent_black());
    let mut quad = gpui::fill(bounds, fill).corner_radii(px(radius));
    if let Some(stroke) = shape.stroke {
        quad = quad
            .border_widths(px(shape.stroke_width))
            .border_color(stroke);
    }
    window.paint_quad(quad);
}

fn trace(
    builder: &mut gpui::PathBuilder,
    segments: &[Segment],
    at: &impl Fn(f32, f32) -> Point<Pixels>,
) {
    for segment in segments {
        match *segment {
            Segment::Move(x, y) => builder.move_to(at(x, y)),
            Segment::Line(x, y) => builder.line_to(at(x, y)),
            Segment::Cubic(x1, y1, x2, y2, x, y) => {
                builder.cubic_bezier_to(at(x, y), at(x1, y1), at(x2, y2))
            }
            Segment::Quad(x1, y1, x, y) => builder.curve_to(at(x, y), at(x1, y1)),
            Segment::Close => builder.close(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(tag: &str, attrs: &[(&str, &str)]) -> Node {
        Node::Element {
            tag: tag.into(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            children: vec![],
        }
    }

    #[test]
    fn reads_shapes() {
        let theme: Theme = [("primary".to_string(), "#ff0000".to_string())].into();
        let shapes = shapes(
            &[
                el(
                    "rect",
                    &[
                        ("x", "1"),
                        ("y", "2"),
                        ("width", "3"),
                        ("height", "4"),
                        ("class", "fill-primary"),
                    ],
                ),
                Node::Text("ignored".into()),
                el(
                    "circle",
                    &[
                        ("cx", "5"),
                        ("cy", "5"),
                        ("r", "2"),
                        ("class", "stroke-primary"),
                        ("stroke-width", "3"),
                    ],
                ),
                el("polyline", &[("points", "0,0 10,10 20,5")]),
                el("blink", &[]),
            ],
            &theme,
        );
        assert_eq!(shapes.len(), 3);
        assert_eq!(
            shapes[0].geometry,
            Geometry::Rect {
                x: 1.,
                y: 2.,
                width: 3.,
                height: 4.,
                radius: 0.
            }
        );
        assert!(shapes[0].fill.is_some() && shapes[0].stroke.is_none());
        assert_eq!((shapes[1].fill, shapes[1].stroke_width), (None, 3.));
        assert_eq!(
            shapes[2].geometry,
            Geometry::Path(vec![
                Segment::Move(0., 0.),
                Segment::Line(10., 10.),
                Segment::Line(20., 5.)
            ])
        );
        // An open path with no colour is stroked, as SVG lines are.
        assert!(shapes[2].stroke.is_some() && shapes[2].fill.is_none());
    }

    #[test]
    fn reads_path_data() {
        assert_eq!(
            path("M0 0L10,10 H 20 V5 C 1 2 3 4 5 6 Q 7 8 9 10 Z"),
            vec![
                Segment::Move(0., 0.),
                Segment::Line(10., 10.),
                Segment::Line(20., 10.),
                Segment::Line(20., 5.),
                Segment::Cubic(1., 2., 3., 4., 5., 6.),
                Segment::Quad(7., 8., 9., 10.),
                Segment::Close,
            ]
        );
        // Stops at what it doesn't know (relative commands, arcs).
        assert_eq!(path("M 1 1 l 2 2 L 5 5"), vec![Segment::Move(1., 1.)]);
        assert_eq!(path("M 1"), vec![]);
    }
}
