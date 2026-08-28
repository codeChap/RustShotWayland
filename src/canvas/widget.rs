//! Widget-stamp kinds and layout math. Pure functions — `render` consumes the
//! bounds. No allocations on the paint path.

use super::geometry::{Bounds, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    Button,
    Input,
    ImageX,
    Checkbox,
    Toggle,
    Measure,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 6] = [
        WidgetKind::Button,
        WidgetKind::Input,
        WidgetKind::ImageX,
        WidgetKind::Checkbox,
        WidgetKind::Toggle,
        WidgetKind::Measure,
    ];
}

/// Corner radius for Button / Input. Spec: `min(8, h/2, w/2)`.
pub fn corner_radius(b: Bounds) -> f32 {
    8.0_f32.min(b.h * 0.5).min(b.w * 0.5).max(0.0)
}

/// Inner placeholder bar for Input.
pub fn input_placeholder(b: Bounds) -> Bounds {
    let h = (b.h * 0.22).clamp(3.0, 10.0).min(b.h.max(0.0));
    let w = (b.w * 0.60).max(0.0);
    Bounds {
        x: b.x + b.w * 0.12,
        y: b.y + (b.h - h) * 0.5,
        w,
        h,
    }
}

/// Square checkbox, top-left of the drag. Stretched rects still read as a box.
pub fn checkbox_square(b: Bounds) -> Bounds {
    let s = b.w.min(b.h).max(0.0);
    Bounds {
        x: b.x,
        y: b.y,
        w: s,
        h: s,
    }
}

/// Classic tick: mid-left → lower-mid → upper-right.
pub fn checkbox_tick(sq: Bounds) -> (Pos, Pos, Pos) {
    (
        Pos {
            x: sq.x + sq.w * 0.22,
            y: sq.y + sq.h * 0.52,
        },
        Pos {
            x: sq.x + sq.w * 0.42,
            y: sq.y + sq.h * 0.74,
        },
        Pos {
            x: sq.x + sq.w * 0.78,
            y: sq.y + sq.h * 0.28,
        },
    )
}

/// Knob on the right ("on") so it reads as a switch, not a pill button.
pub fn toggle_knob(b: Bounds) -> (Pos, f32) {
    let pad = (b.h * 0.14).max(1.0);
    let r = (b.h * 0.5 - pad).max(1.0);
    let cy = b.y + b.h * 0.5;
    let cx = (b.x + b.w - b.h * 0.5).max(b.x + r);
    (Pos { x: cx, y: cy }, r)
}

pub fn measure_px(b: Bounds) -> (u32, u32) {
    (b.w.round().max(0.0) as u32, b.h.round().max(0.0) as u32)
}

/// Strip-glyph box inside a circular tool button of diameter `d`.
pub fn glyph_bounds(kind: WidgetKind, cx: f32, cy: f32, d: f32) -> Bounds {
    let (w, h) = match kind {
        WidgetKind::Checkbox => {
            let s = d * 0.42;
            (s, s)
        }
        WidgetKind::Toggle => (d * 0.58, d * 0.30),
        WidgetKind::Measure => (d * 0.50, d * 0.38),
        _ => (d * 0.54, d * 0.40),
    };
    Bounds::centered(cx, cy, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(w: f32, h: f32) -> Bounds {
        Bounds {
            x: 10.0,
            y: 20.0,
            w,
            h,
        }
    }

    #[test]
    fn corner_radius_clamps_to_eight_and_half_sides() {
        assert_eq!(corner_radius(b(100.0, 40.0)), 8.0);
        assert_eq!(corner_radius(b(10.0, 10.0)), 5.0);
        assert_eq!(corner_radius(b(4.0, 100.0)), 2.0);
        assert_eq!(corner_radius(b(0.0, 10.0)), 0.0);
    }

    #[test]
    fn input_placeholder_is_inset_and_vertically_centered() {
        let outer = b(100.0, 40.0);
        let p = input_placeholder(outer);
        assert!((p.x - 22.0).abs() < 0.01, "12% left pad, got x={}", p.x);
        assert!((p.w - 60.0).abs() < 0.01, "60% width, got w={}", p.w);
        assert!((p.h - 8.8).abs() < 0.01, "22% of 40 = 8.8, got h={}", p.h);
        let mid = p.y + p.h * 0.5;
        assert!((mid - (20.0 + 20.0)).abs() < 0.01, "vertically centered");
    }

    #[test]
    fn input_placeholder_clamps_bar_height() {
        let tall = input_placeholder(b(100.0, 80.0));
        assert_eq!(tall.h, 10.0);
        let short = input_placeholder(b(100.0, 10.0));
        assert_eq!(short.h, 3.0);
    }

    #[test]
    fn widgetkind_all_has_six() {
        assert_eq!(WidgetKind::ALL.len(), 6);
    }

    #[test]
    fn glyph_bounds_is_centered_in_the_button() {
        let g = glyph_bounds(WidgetKind::Button, 0.0, 0.0, 34.0);
        assert!((g.x + g.w * 0.5).abs() < 0.01);
        assert!((g.y + g.h * 0.5).abs() < 0.01);
        assert!((g.w - 34.0 * 0.54).abs() < 0.01);
        let sq = glyph_bounds(WidgetKind::Checkbox, 0.0, 0.0, 34.0);
        assert!((sq.w - sq.h).abs() < 0.01);
    }

    #[test]
    fn checkbox_square_uses_the_short_side() {
        let sq = checkbox_square(b(80.0, 30.0));
        assert_eq!(sq.w, 30.0);
        assert_eq!(sq.h, 30.0);
        assert_eq!(sq.x, 10.0);
        assert_eq!(sq.y, 20.0);
    }

    #[test]
    fn checkbox_tick_stays_inside_the_square() {
        let sq = checkbox_square(b(40.0, 40.0));
        let (a, m, c) = checkbox_tick(sq);
        for p in [a, m, c] {
            assert!(p.x >= sq.x && p.x <= sq.x + sq.w);
            assert!(p.y >= sq.y && p.y <= sq.y + sq.h);
        }
        assert!(m.y > a.y && c.x > m.x);
    }

    #[test]
    fn toggle_knob_sits_on_the_right() {
        let outer = b(80.0, 30.0);
        let (p, r) = toggle_knob(outer);
        assert!(r > 0.0);
        assert!(p.x > outer.x + outer.w * 0.5, "knob should be on the right");
        assert!(p.x + r <= outer.x + outer.w + 0.5);
    }

    #[test]
    fn measure_px_rounds_the_drag() {
        assert_eq!(measure_px(b(120.4, 79.6)), (120, 80));
        assert_eq!(measure_px(b(0.0, 10.0)), (0, 10));
    }
}
