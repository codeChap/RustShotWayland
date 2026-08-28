//! Selection rectangle editing — handle hit-testing, resize math, and the
//! little yellow-outlined squares drawn at corners + edge midpoints.

use crate::canvas::{Bounds, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SelectionEdit {
    None,
    Moving,
    Resizing(Handle),
}

/// Size of the drawn handle square, in px.
pub(super) const HANDLE_VISUAL: f32 = 10.0;

/// Grab zones, anchored to what's actually on screen so "can grab" stays close
/// to "looks grabbable". The handle square is `HANDLE_VISUAL` across (so ±5 from
/// the corner) and the frame stroke is 2px wide. These were 14/8 — roughly 3x
/// the visual — which is why the frame grabbed from well off the yellow line.
pub(super) const CORNER_HIT: f32 = HANDLE_VISUAL * 0.5 + 2.0;
pub(super) const EDGE_HIT: f32 = 4.0;

/// Returns which handle (if any) the pointer is on. Corners take priority over
/// edges, and clicking anywhere along an edge line counts as a grab — not just
/// the midpoint dot.
pub(super) fn handle_at(rect: Bounds, p: Pos) -> Option<Handle> {
    let l = rect.x;
    let r = rect.x + rect.w;
    let t = rect.y;
    let b = rect.y + rect.h;

    let in_corner =
        |cx: f32, cy: f32| (p.x - cx).abs() <= CORNER_HIT && (p.y - cy).abs() <= CORNER_HIT;
    if in_corner(l, t) {
        return Some(Handle::NW);
    }
    if in_corner(r, t) {
        return Some(Handle::NE);
    }
    if in_corner(l, b) {
        return Some(Handle::SW);
    }
    if in_corner(r, b) {
        return Some(Handle::SE);
    }

    let in_x = p.x >= l - EDGE_HIT && p.x <= r + EDGE_HIT;
    let in_y = p.y >= t - EDGE_HIT && p.y <= b + EDGE_HIT;
    if (p.y - t).abs() <= EDGE_HIT && in_x {
        return Some(Handle::N);
    }
    if (p.y - b).abs() <= EDGE_HIT && in_x {
        return Some(Handle::S);
    }
    if (p.x - l).abs() <= EDGE_HIT && in_y {
        return Some(Handle::W);
    }
    if (p.x - r).abs() <= EDGE_HIT && in_y {
        return Some(Handle::E);
    }
    None
}

/// Apply a drag delta to the rect for the given handle. Normalizes min/max
/// so dragging past the opposite edge flips cleanly.
pub(super) fn resize_rect(rect: Bounds, handle: Handle, dx: f32, dy: f32) -> Bounds {
    let mut l = rect.x;
    let mut t = rect.y;
    let mut r = rect.x + rect.w;
    let mut b = rect.y + rect.h;
    match handle {
        Handle::N => t += dy,
        Handle::S => b += dy,
        Handle::E => r += dx,
        Handle::W => l += dx,
        Handle::NE => {
            t += dy;
            r += dx;
        }
        Handle::NW => {
            t += dy;
            l += dx;
        }
        Handle::SE => {
            r += dx;
            b += dy;
        }
        Handle::SW => {
            l += dx;
            b += dy;
        }
    }
    if l > r {
        std::mem::swap(&mut l, &mut r);
    }
    if t > b {
        std::mem::swap(&mut t, &mut b);
    }
    Bounds {
        x: l,
        y: t,
        w: r - l,
        h: b - t,
    }
}

pub(super) fn handle_corner_positions(rect: Bounds) -> [(Handle, f32, f32); 8] {
    let cx = rect.x + rect.w * 0.5;
    let cy = rect.y + rect.h * 0.5;
    let l = rect.x;
    let r = rect.x + rect.w;
    let t = rect.y;
    let b = rect.y + rect.h;
    [
        (Handle::NW, l, t),
        (Handle::N, cx, t),
        (Handle::NE, r, t),
        (Handle::E, r, cy),
        (Handle::SE, r, b),
        (Handle::S, cx, b),
        (Handle::SW, l, b),
        (Handle::W, l, cy),
    ]
}

/// Stock X11 cursor-font glyph ID for each resize direction.
/// See `/usr/include/X11/cursorfont.h`.
pub(super) fn cursor_glyph_for_handle(h: Handle) -> u16 {
    match h {
        Handle::N => 138,  // XC_top_side
        Handle::S => 16,   // XC_bottom_side
        Handle::E => 96,   // XC_right_side
        Handle::W => 70,   // XC_left_side
        Handle::NE => 136, // XC_top_right_corner
        Handle::NW => 134, // XC_top_left_corner
        Handle::SE => 14,  // XC_bottom_right_corner
        Handle::SW => 12,  // XC_bottom_left_corner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn handle_at_corners_take_priority() {
        let r = rect(100.0, 100.0, 200.0, 150.0);
        // Near NW corner
        assert_eq!(handle_at(r, Pos { x: 102.0, y: 102.0 }), Some(Handle::NW));
        // Near SE corner
        assert_eq!(handle_at(r, Pos { x: 295.0, y: 245.0 }), Some(Handle::SE));
    }

    #[test]
    fn handle_at_edges() {
        let r = rect(50.0, 50.0, 100.0, 80.0);
        // Top edge (not corner)
        assert_eq!(handle_at(r, Pos { x: 90.0, y: 50.0 }), Some(Handle::N));
        // Right edge
        assert_eq!(handle_at(r, Pos { x: 150.0, y: 90.0 }), Some(Handle::E));
        // Bottom edge
        assert_eq!(handle_at(r, Pos { x: 80.0, y: 130.0 }), Some(Handle::S));
    }

    #[test]
    fn handle_at_edge_hit_tolerance() {
        let r = rect(0.0, 0.0, 100.0, 100.0);
        // Just inside edge tolerance on top
        assert_eq!(handle_at(r, Pos { x: 50.0, y: -3.0 }), Some(Handle::N));
        // Just outside
        assert!(handle_at(r, Pos { x: 50.0, y: -6.0 }).is_none());
    }

    #[test]
    fn handle_at_does_not_grab_from_far_off_the_line() {
        // Guards the "borders grab too loosely" fix: these distances used to
        // count as a grab under the old 14/8 tolerances and must not now.
        let r = rect(0.0, 0.0, 100.0, 100.0);
        assert!(
            handle_at(r, Pos { x: 50.0, y: 7.0 }).is_none(),
            "7px in from the top edge is not an edge grab"
        );
        assert!(
            handle_at(r, Pos { x: 12.0, y: 12.0 }).is_none(),
            "12px diagonally off the NW corner is not a corner grab"
        );
    }

    #[test]
    fn handle_at_still_grabs_on_the_line() {
        // The flip side: tightening must not make the frame ungrabbable.
        let r = rect(0.0, 0.0, 100.0, 100.0);
        assert_eq!(handle_at(r, Pos { x: 0.0, y: 0.0 }), Some(Handle::NW));
        assert_eq!(handle_at(r, Pos { x: 50.0, y: 0.0 }), Some(Handle::N));
        assert_eq!(handle_at(r, Pos { x: 100.0, y: 50.0 }), Some(Handle::E));
        // Within the drawn handle square (±5 of the corner) still grabs.
        assert_eq!(handle_at(r, Pos { x: 4.0, y: 4.0 }), Some(Handle::NW));
    }

    #[test]
    fn handle_at_inside_no_handle() {
        let r = rect(10.0, 10.0, 50.0, 50.0);
        assert!(handle_at(r, Pos { x: 30.0, y: 30.0 }).is_none());
    }

    #[test]
    fn resize_rect_cardinal_directions() {
        let r = rect(100.0, 100.0, 50.0, 40.0);

        let moved_n = resize_rect(r, Handle::N, 0.0, -10.0);
        assert_eq!(moved_n.y, 90.0);
        assert_eq!(moved_n.h, 50.0);

        let moved_e = resize_rect(r, Handle::E, 15.0, 0.0);
        assert_eq!(moved_e.w, 65.0);
    }

    #[test]
    fn resize_rect_crossing_flips_cleanly() {
        let r = rect(0.0, 0.0, 100.0, 80.0);
        // Drag N handle way past the bottom
        let result = resize_rect(r, Handle::N, 0.0, 200.0);
        // After flip, top becomes bottom
        assert!(result.y <= 80.0);
        assert!(result.h > 0.0);
    }

    #[test]
    fn resize_rect_diagonal() {
        let r = rect(20.0, 30.0, 60.0, 40.0);
        let res = resize_rect(r, Handle::SE, 10.0, 5.0);
        assert_eq!(res.w, 70.0);
        assert_eq!(res.h, 45.0);
    }

    #[test]
    fn handle_corner_positions_returns_all_eight() {
        let r = rect(0.0, 0.0, 100.0, 100.0);
        let corners = handle_corner_positions(r);
        assert_eq!(corners.len(), 8);
        // Spot-check a couple
        let has_nw = corners.iter().any(|(h, _, _)| *h == Handle::NW);
        let has_s = corners.iter().any(|(h, _, _)| *h == Handle::S);
        assert!(has_nw && has_s);
    }

    #[test]
    fn cursor_glyphs_are_distinct() {
        use std::collections::HashSet;
        let glyphs: HashSet<_> = [
            Handle::N,
            Handle::S,
            Handle::E,
            Handle::W,
            Handle::NE,
            Handle::NW,
            Handle::SE,
            Handle::SW,
        ]
        .iter()
        .map(|&h| cursor_glyph_for_handle(h))
        .collect();
        // All 8 should be unique glyph IDs
        assert_eq!(glyphs.len(), 8);
    }
}
