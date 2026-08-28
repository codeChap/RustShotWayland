//! In-progress annotations. A Draft is alive during a drag and becomes an
//! `Annotation` on release (via `finalize`).

use crate::canvas::{Annotation, Bounds, Pos, Style, ToolKind, WidgetKind};
use image::Rgba;

/// Highlighter uses a fixed semi-transparent yellow + wide stroke, like a
/// physical marker — ignores `canvas.style`.
const HIGHLIGHTER_STYLE: Style = Style {
    color: Rgba([255, 230, 0, 110]),
    width: 20.0,
};

#[derive(Debug, Clone)]
pub(super) enum Draft {
    Pencil {
        points: Vec<Pos>,
        style: Style,
    },
    Line {
        start: Pos,
        end: Pos,
        style: Style,
    },
    Arrow {
        start: Pos,
        end: Pos,
        style: Style,
    },
    Rect {
        start: Pos,
        end: Pos,
        style: Style,
    },
    Ellipse {
        start: Pos,
        end: Pos,
        style: Style,
    },
    Pixelate {
        start: Pos,
        end: Pos,
        block: u32,
    },
    Spotlight {
        start: Pos,
        end: Pos,
    },
    Widget {
        kind: WidgetKind,
        start: Pos,
        end: Pos,
        style: Style,
    },
}

impl Draft {
    /// Build a draft for `tool` starting at `pos`. Returns `None` for tools that
    /// don't use drag (e.g. Counter fires on click, not drag).
    pub(super) fn new(tool: ToolKind, pos: Pos, style: Style, pixelate_block: u32) -> Option<Self> {
        Some(match tool {
            ToolKind::Pencil => Draft::Pencil {
                points: vec![pos],
                style,
            },
            ToolKind::Highlighter => Draft::Pencil {
                points: vec![pos],
                style: HIGHLIGHTER_STYLE,
            },
            ToolKind::Line => Draft::Line {
                start: pos,
                end: pos,
                style,
            },
            ToolKind::Arrow => Draft::Arrow {
                start: pos,
                end: pos,
                style,
            },
            ToolKind::Rect => Draft::Rect {
                start: pos,
                end: pos,
                style,
            },
            ToolKind::Ellipse => Draft::Ellipse {
                start: pos,
                end: pos,
                style,
            },
            ToolKind::Pixelate => Draft::Pixelate {
                start: pos,
                end: pos,
                block: pixelate_block,
            },
            ToolKind::Spotlight => Draft::Spotlight {
                start: pos,
                end: pos,
            },
            ToolKind::Widget(kind) => Draft::Widget {
                kind,
                start: pos,
                end: pos,
                style,
            },
            // Click-to-place — placement happens in on_press, no drag draft.
            ToolKind::Counter => return None,
        })
    }

    /// Update the in-progress shape, optionally constraining line tools to 10°
    /// increments (Shift). Pencil / highlighter collapse to a straight segment
    /// from the stroke start. Rect-ish tools keep their own geometry.
    /// `p` is expected to already sit inside `bounds`.
    pub(super) fn extend_snapped(&mut self, p: Pos, bounds: Bounds, snap: bool) {
        if snap {
            match self {
                Draft::Line { start, end, .. } | Draft::Arrow { start, end, .. } => {
                    *end = snap_degrees(*start, p, bounds, 10.0);
                    return;
                }
                Draft::Pencil { points, .. } => {
                    if let Some(&start) = points.first() {
                        let snapped = snap_degrees(start, p, bounds, 10.0);
                        points.clear();
                        points.push(start);
                        points.push(snapped);
                        return;
                    }
                }
                _ => {}
            }
        }
        self.extend(p);
    }

    /// Called every frame during a drag to update the in-progress shape.
    pub(super) fn extend(&mut self, p: Pos) {
        match self {
            Draft::Pencil { points, .. } => points.push(p),
            Draft::Line { end, .. }
            | Draft::Arrow { end, .. }
            | Draft::Rect { end, .. }
            | Draft::Ellipse { end, .. }
            | Draft::Pixelate { end, .. }
            | Draft::Spotlight { end, .. }
            | Draft::Widget { end, .. } => *end = p,
        }
    }

    /// Convert a completed draft into a committed `Annotation`. Returns `None`
    /// for zero-area drags (so a click-without-drag doesn't create a garbage shape).
    pub(super) fn finalize(self) -> Option<Annotation> {
        match self {
            Draft::Pencil { points, style } if points.len() >= 2 => Some(Annotation::Pencil {
                points,
                color: style.color,
                width: style.width,
            }),
            Draft::Pencil { .. } => None,
            Draft::Line { start, end, style } => {
                (dist2(start, end) >= 4.0).then_some(Annotation::Line {
                    start,
                    end,
                    color: style.color,
                    width: style.width,
                })
            }
            Draft::Arrow { start, end, style } => {
                (dist2(start, end) >= 4.0).then_some(Annotation::Arrow {
                    start,
                    end,
                    color: style.color,
                    width: style.width,
                })
            }
            Draft::Rect { start, end, style } => {
                drawable(start, end).map(|rect| Annotation::Rect {
                    rect,
                    color: style.color,
                    width: style.width,
                })
            }
            Draft::Ellipse { start, end, style } => {
                drawable(start, end).map(|rect| Annotation::Ellipse {
                    rect,
                    color: style.color,
                    width: style.width,
                })
            }
            Draft::Pixelate { start, end, block } => {
                drawable(start, end).map(|rect| Annotation::Pixelate { rect, block })
            }
            Draft::Spotlight { start, end } => {
                drawable(start, end).map(|rect| Annotation::Spotlight { rect })
            }
            Draft::Widget {
                kind,
                start,
                end,
                style,
            } => drawable(start, end).map(|rect| Annotation::Widget {
                kind,
                rect,
                color: style.color,
                width: style.width,
            }),
        }
    }

    /// Screen-space AABB of the in-progress shape, if it has any area.
    pub(super) fn bounds(&self) -> Option<Bounds> {
        match self {
            Draft::Pencil { points, .. } => {
                let p0 = *points.first()?;
                let mut min = p0;
                let mut max = p0;
                for p in points.iter().skip(1) {
                    min.x = min.x.min(p.x);
                    min.y = min.y.min(p.y);
                    max.x = max.x.max(p.x);
                    max.y = max.y.max(p.y);
                }
                Some(Bounds::from_two(min, max))
            }
            Draft::Line { start, end, .. }
            | Draft::Arrow { start, end, .. }
            | Draft::Rect { start, end, .. }
            | Draft::Ellipse { start, end, .. }
            | Draft::Pixelate { start, end, .. }
            | Draft::Spotlight { start, end, .. }
            | Draft::Widget { start, end, .. } => Some(Bounds::from_two(*start, *end)),
        }
    }
}

/// Snap `p` onto the nearest `step_deg` ray from `anchor`, keeping the result
/// inside `bounds`.
///
/// Length follows the cursor, shortened where the ray would leave the
/// selection. Clamping the snapped *point* to the box instead would bend the
/// line off-grid exactly when it hits an edge.
fn snap_degrees(anchor: Pos, p: Pos, bounds: Bounds, step_deg: f32) -> Pos {
    let dx = p.x - anchor.x;
    let dy = p.y - anchor.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return p;
    }
    let step = step_deg.to_radians();
    if step <= 0.0 {
        return p;
    }
    let angle = (dy.atan2(dx) / step).round() * step;
    let (ux, uy) = (angle.cos(), angle.sin());

    // Walk the ray out to the first edge it crosses. `anchor` is always inside
    // the selection (on_press only starts a draft there), so every bound below
    // yields a non-negative distance.
    let mut max = f32::INFINITY;
    if ux > 1e-6 {
        max = max.min((bounds.x + bounds.w - anchor.x) / ux);
    } else if ux < -1e-6 {
        max = max.min((bounds.x - anchor.x) / ux);
    }
    if uy > 1e-6 {
        max = max.min((bounds.y + bounds.h - anchor.y) / uy);
    } else if uy < -1e-6 {
        max = max.min((bounds.y - anchor.y) / uy);
    }
    let len = len.min(max.max(0.0));

    Pos {
        x: anchor.x + ux * len,
        y: anchor.y + uy * len,
    }
}

/// Returns a `Bounds` only if the rect has enough area to be visible.
fn drawable(a: Pos, b: Pos) -> Option<Bounds> {
    let r = Bounds::from_two(a, b);
    (r.w >= 2.0 && r.h >= 2.0).then_some(r)
}

fn dist2(a: Pos, b: Pos) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::ToolKind;
    use image::Rgba;

    fn style() -> Style {
        Style {
            color: Rgba([255, 0, 0, 255]),
            width: 4.0,
        }
    }

    #[test]
    fn draft_new_drag_tools_create_drafts() {
        let p = Pos { x: 10.0, y: 20.0 };
        assert!(Draft::new(ToolKind::Line, p, style(), 8).is_some());
        assert!(Draft::new(ToolKind::Rect, p, style(), 8).is_some());
        assert!(Draft::new(ToolKind::Pixelate, p, style(), 8).is_some());
        assert!(Draft::new(ToolKind::Spotlight, p, style(), 8).is_some());
        for &k in &WidgetKind::ALL {
            assert!(Draft::new(ToolKind::Widget(k), p, style(), 8).is_some());
        }
    }

    #[test]
    fn draft_new_click_tools_return_none() {
        let p = Pos { x: 0.0, y: 0.0 };
        assert!(Draft::new(ToolKind::Counter, p, style(), 8).is_none());
    }

    #[test]
    fn draft_finalize_rejects_too_small_line() {
        let mut d = Draft::new(ToolKind::Line, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 1.0, y: 1.0 }); // dist2 = 2 < 4.0
        assert!(d.finalize().is_none());
    }

    #[test]
    fn draft_finalize_accepts_reasonable_line() {
        let mut d = Draft::new(ToolKind::Arrow, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 10.0, y: 0.0 });
        let ann = d.finalize().expect("should produce annotation");
        match ann {
            Annotation::Arrow { start, end, .. } => {
                assert_eq!(start.x, 0.0);
                assert_eq!(end.x, 10.0);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn draft_finalize_pixelate_and_rect_require_min_area() {
        let mut d = Draft::new(ToolKind::Rect, Pos { x: 5.0, y: 5.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 6.0, y: 6.0 }); // w=1, h=1 → below 2.0 threshold
        assert!(d.finalize().is_none());

        let mut d2 = Draft::new(ToolKind::Pixelate, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        d2.extend(Pos { x: 1.0, y: 1.0 }); // w=1, h=1
        assert!(d2.finalize().is_none());
    }

    #[test]
    fn draft_pencil_requires_two_points() {
        let mut d = Draft::new(ToolKind::Pencil, Pos { x: 1.0, y: 1.0 }, style(), 8).unwrap();
        assert!(d.clone().finalize().is_none()); // only one point
        d.extend(Pos { x: 2.0, y: 2.0 });
        assert!(d.finalize().is_some());
    }

    /// Generous bounds so snapping isn't length-clamped unless a test wants it.
    fn big() -> Bounds {
        Bounds {
            x: -1000.0,
            y: -1000.0,
            w: 2000.0,
            h: 2000.0,
        }
    }

    fn line_from(x: f32, y: f32) -> Draft {
        Draft::new(ToolKind::Line, Pos { x, y }, style(), 8).unwrap()
    }

    fn end_of(d: &Draft) -> Pos {
        match d {
            Draft::Line { end, .. } | Draft::Arrow { end, .. } => *end,
            _ => panic!("not a two-point draft"),
        }
    }

    #[test]
    fn snap_pulls_a_near_horizontal_drag_flat() {
        let mut d = line_from(0.0, 0.0);
        // 100 across, 4 down (~2.3°) — nearer 0° than 10°.
        d.extend_snapped(Pos { x: 100.0, y: 4.0 }, big(), true);
        let e = end_of(&d);
        assert!(e.y.abs() < 0.01, "expected flat, got y={}", e.y);
        assert!(e.x > 99.0, "length should follow the cursor, got x={}", e.x);
    }

    #[test]
    fn snap_holds_a_true_10_degree_ray() {
        let mut d = line_from(0.0, 0.0);
        // atan2(17.633, 100) = 10°. Stay on that ray.
        d.extend_snapped(
            Pos {
                x: 100.0,
                y: 17.633,
            },
            big(),
            true,
        );
        let e = end_of(&d);
        let deg = e.y.atan2(e.x).to_degrees();
        assert!(
            (deg - 10.0).abs() < 0.05,
            "expected 10°, got {deg} at ({}, {})",
            e.x,
            e.y
        );
    }

    #[test]
    fn snap_rounds_near_40_not_45() {
        let mut d = line_from(0.0, 0.0);
        // atan2(70, 80) ≈ 41.2° → 40° on a 10° grid (not 45°).
        d.extend_snapped(Pos { x: 80.0, y: 70.0 }, big(), true);
        let e = end_of(&d);
        let deg = e.y.atan2(e.x).to_degrees();
        assert!(
            (deg - 40.0).abs() < 0.05,
            "expected 40°, got {deg} at ({}, {})",
            e.x,
            e.y
        );
    }

    #[test]
    fn snap_off_leaves_the_point_alone() {
        let mut d = line_from(0.0, 0.0);
        d.extend_snapped(Pos { x: 100.0, y: 9.0 }, big(), false);
        let e = end_of(&d);
        assert_eq!((e.x, e.y), (100.0, 9.0));
    }

    #[test]
    fn snap_shortens_rather_than_bending_at_the_edge() {
        // Anchor at origin, selection ends at x=50. A 40° ray (nearest 10° to
        // the 43° cursor) must stop at the right edge, still at 40°.
        let bounds = Bounds {
            x: 0.0,
            y: 0.0,
            w: 50.0,
            h: 50.0,
        };
        let mut d = line_from(0.0, 0.0);
        d.extend_snapped(Pos { x: 200.0, y: 190.0 }, bounds, true);
        let e = end_of(&d);
        let deg = e.y.atan2(e.x).to_degrees();
        assert!(
            (deg - 40.0).abs() < 0.05,
            "must stay 40°, got {deg} at ({}, {})",
            e.x,
            e.y
        );
        assert!(
            e.x <= 50.01 && e.y <= 50.01,
            "must stay in bounds, got ({}, {})",
            e.x,
            e.y
        );
        assert!(e.x > 49.0, "should reach the edge, got x={}", e.x);
    }

    #[test]
    fn snap_pencil_collapses_to_a_straight_10_degree_segment() {
        let mut pencil = Draft::new(ToolKind::Pencil, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        pencil.extend(Pos { x: 10.0, y: 8.0 });
        pencil.extend_snapped(Pos { x: 100.0, y: 4.0 }, big(), true);
        match pencil {
            Draft::Pencil { points, .. } => {
                assert_eq!(points.len(), 2);
                assert!(
                    points[1].y.abs() < 0.01,
                    "pencil+shift should flatten, y={}",
                    points[1].y
                );
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn snap_ignores_area_tools() {
        let mut r = Draft::new(ToolKind::Rect, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        r.extend_snapped(Pos { x: 100.0, y: 9.0 }, big(), true);
        match r {
            Draft::Rect { end, .. } => assert_eq!(end.y, 9.0, "rect must not snap"),
            _ => panic!("wrong variant"),
        }

        let mut w = Draft::new(
            ToolKind::Widget(WidgetKind::Button),
            Pos { x: 0.0, y: 0.0 },
            style(),
            8,
        )
        .unwrap();
        w.extend_snapped(Pos { x: 100.0, y: 9.0 }, big(), true);
        match w {
            Draft::Widget { end, .. } => assert_eq!(end.y, 9.0, "widget must not snap"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn draft_finalize_widget_rejects_tiny_and_keeps_kind() {
        let mut tiny = Draft::new(
            ToolKind::Widget(WidgetKind::Input),
            Pos { x: 5.0, y: 5.0 },
            style(),
            8,
        )
        .unwrap();
        tiny.extend(Pos { x: 6.0, y: 6.0 });
        assert!(tiny.finalize().is_none());

        let mut d = Draft::new(
            ToolKind::Widget(WidgetKind::ImageX),
            Pos { x: 0.0, y: 0.0 },
            style(),
            8,
        )
        .unwrap();
        d.extend(Pos { x: 40.0, y: 30.0 });
        match d.finalize().expect("widget drag") {
            Annotation::Widget { kind, rect, .. } => {
                assert_eq!(kind, WidgetKind::ImageX);
                assert!(rect.w >= 2.0 && rect.h >= 2.0);
            }
            _ => panic!("expected Annotation::Widget"),
        }
    }

    #[test]
    fn draft_bounds_covers_line() {
        let mut d = Draft::new(ToolKind::Line, Pos { x: 10.0, y: 20.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 40.0, y: 50.0 });
        let b = d.bounds().expect("line bounds");
        assert_eq!(b.x, 10.0);
        assert_eq!(b.y, 20.0);
        assert_eq!(b.w, 30.0);
        assert_eq!(b.h, 30.0);
    }

    #[test]
    fn spotlight_drag_follows_cursor_like_rect() {
        let mut d = Draft::new(ToolKind::Spotlight, Pos { x: 10.0, y: 10.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 50.0, y: 30.0 });
        match d.finalize().expect("spotlight") {
            Annotation::Spotlight { rect } => {
                assert_eq!(rect.w, 40.0);
                assert_eq!(rect.h, 20.0);
            }
            _ => panic!("expected Spotlight"),
        }
    }

    #[test]
    fn draft_finalize_spotlight_requires_min_area() {
        let mut tiny = Draft::new(ToolKind::Spotlight, Pos { x: 5.0, y: 5.0 }, style(), 8).unwrap();
        tiny.extend(Pos { x: 6.0, y: 6.0 });
        assert!(tiny.finalize().is_none());

        let mut d = Draft::new(ToolKind::Spotlight, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        d.extend(Pos { x: 40.0, y: 30.0 });
        match d.finalize().expect("spotlight") {
            Annotation::Spotlight { rect } => {
                assert!(rect.w >= 2.0 && rect.h >= 2.0);
            }
            _ => panic!("expected Spotlight"),
        }
    }

    #[test]
    fn highlighter_uses_special_style() {
        let d = Draft::new(ToolKind::Highlighter, Pos { x: 0.0, y: 0.0 }, style(), 8).unwrap();
        match d {
            Draft::Pencil { style, .. } => {
                assert_eq!(style.width, 20.0);
                assert_eq!(style.color.0[1], 230); // yellow-ish
            }
            _ => panic!("highlighter should produce Pencil draft internally"),
        }
    }
}
