//! Pointer/keyboard interaction: press, motion, release, keys, cursor glyph.

use super::draft::Draft;
use super::selection::{cursor_glyph_for_handle, handle_at, resize_rect, SelectionEdit};
use super::state::{Mode, OverlayState};
use super::tool_buttons::{self, Hit};
use super::wl_win::{
    KS_1, KS_8, KS_C_LOWER, KS_ESCAPE, KS_KP_ENTER, KS_RETURN, KS_Y_LOWER, KS_Z_LOWER,
    XC_CROSSHAIR, XC_FLEUR, XC_HAND1, XC_LEFT_PTR,
};
use crate::canvas::{Annotation, Bounds, Pos, ToolKind};
use crate::ui::UiResult;

/// Click-vs-drag threshold in pixels squared. Motion below this on release
/// counts as a click (used for Counter placement + strip clicks).
const CLICK_SQ: f32 = 4.0 * 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Dragging {
    None,
    Region,
    Draft,
    EditResize,
    EditMove,
    Strip(Hit),
}

pub(super) fn on_press(state: &mut OverlayState, p: Pos) -> Dragging {
    if let Some(sel) = state.selection {
        let strip = tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
        if tool_buttons::contains(strip, p) {
            if let Some(hit) = tool_buttons::hit(strip, p) {
                return Dragging::Strip(hit);
            }
            // Inside strip but between buttons — eat the click, no drag.
            return Dragging::None;
        }
    }

    match state.mode {
        Mode::SelectingRegion => {
            state.sel_drag_start = Some(p);
            state.selection = None;
            Dragging::Region
        }
        Mode::Annotating => press_annotating(state, p),
    }
}

fn press_annotating(state: &mut OverlayState, p: Pos) -> Dragging {
    let Some(sel) = state.selection else {
        return Dragging::None;
    };

    if let Some(h) = handle_at(sel, p) {
        state.selection_edit = SelectionEdit::Resizing(h);
        state.edit_drag_start = Some(p);
        state.edit_rect_start = Some(sel);
        return Dragging::EditResize;
    }
    // No tool armed, or Ctrl held: inside-drag moves the frame.
    if sel.contains(p) && (state.canvas.tool.is_none() || state.ctrl_down) {
        state.selection_edit = SelectionEdit::Moving;
        state.edit_drag_start = Some(p);
        state.edit_rect_start = Some(sel);
        return Dragging::EditMove;
    }
    if state.canvas.tool == Some(ToolKind::Counter) && sel.contains(p) {
        let n = state.canvas.next_counter();
        state.canvas.push(Annotation::Counter {
            center: p,
            number: n,
            color: state.canvas.style.color,
            radius: state.counter_radius,
        });
        return Dragging::None;
    }
    if let Some(tool) = state.canvas.tool {
        if sel.contains(p) {
            state.draft = Draft::new(tool, p, state.canvas.style, state.pixelate_block);
            return Dragging::Draft;
        }
    }
    Dragging::None
}

/// Refresh tracked modifiers from the event. The press itself may not include
/// the newly held bit; following motion events do, which is what the drag path needs.
pub(super) fn update_mods(state: &mut OverlayState, ctrl: bool, shift: bool) {
    state.ctrl_down = ctrl;
    state.shift_down = shift;
}

pub(super) fn on_motion(state: &mut OverlayState, dragging: &mut Dragging, p: Pos) {
    // Always update strip hover so buttons light up on hover, even outside a drag.
    state.strip_hover = match state.selection {
        Some(sel) => {
            let strip = tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
            if tool_buttons::contains(strip, p) {
                tool_buttons::hit(strip, p)
            } else {
                None
            }
        }
        None => None,
    };

    match *dragging {
        Dragging::Region => {
            if let Some(start) = state.sel_drag_start {
                state.selection = Some(Bounds::from_two(start, p));
            }
        }
        Dragging::Draft => {
            let snap = state.shift_down;
            if let (Some(draft), Some(sel)) = (state.draft.as_mut(), state.selection) {
                draft.extend_snapped(sel.clamp_pos(p), sel, snap);
            }
        }
        Dragging::EditResize => {
            if let (SelectionEdit::Resizing(h), Some(start), Some(rect)) = (
                state.selection_edit,
                state.edit_drag_start,
                state.edit_rect_start,
            ) {
                let dx = p.x - start.x;
                let dy = p.y - start.y;
                state.selection = Some(resize_rect(rect, h, dx, dy));
            }
        }
        Dragging::EditMove => {
            if let (Some(start), Some(rect)) = (state.edit_drag_start, state.edit_rect_start) {
                state.selection = Some(rect.translate(p.x - start.x, p.y - start.y));
            }
        }
        Dragging::None | Dragging::Strip(_) => {}
    }
}

pub(super) fn on_release(
    state: &mut OverlayState,
    dragging: &mut Dragging,
    p: Pos,
    press: Pos,
) -> Option<UiResult> {
    let d = *dragging;
    *dragging = Dragging::None;

    match d {
        Dragging::Region => {
            if let Some(sel) = state.selection {
                if sel.w >= 4.0 && sel.h >= 4.0 {
                    state.mode = Mode::Annotating;
                } else {
                    state.selection = None;
                }
            }
            state.sel_drag_start = None;
        }
        Dragging::Draft => {
            if let Some(draft) = state.draft.take() {
                if let Some(a) = draft.finalize() {
                    state.canvas.push(a);
                }
            }
        }
        Dragging::EditResize | Dragging::EditMove => {
            state.selection_edit = SelectionEdit::None;
            state.edit_drag_start = None;
            state.edit_rect_start = None;
        }
        Dragging::Strip(hit) => {
            // Only trigger if release is still on the same button AND travel < CLICK_SQ.
            let dx = p.x - press.x;
            let dy = p.y - press.y;
            if dx * dx + dy * dy < CLICK_SQ {
                if let Some(sel) = state.selection {
                    let strip =
                        tool_buttons::strip_rect(state.base.width(), state.base.height(), sel);
                    if tool_buttons::hit(strip, p) == Some(hit) {
                        return apply_hit(state, hit);
                    }
                }
            }
        }
        Dragging::None => {}
    }
    None
}

fn apply_hit(state: &mut OverlayState, hit: Hit) -> Option<UiResult> {
    match hit {
        Hit::Tool(t) => {
            // Click the active tool to disarm (back to move-the-frame mode).
            state.canvas.tool = if state.canvas.tool == Some(t) {
                None
            } else {
                Some(t)
            };
            None
        }
        Hit::Save => Some(state.act(false)),
        Hit::Copy => Some(state.act(true)),
    }
}

pub(super) fn handle_key(state: &mut OverlayState, ks: u32, ctrl: bool) -> Option<UiResult> {
    match ks {
        KS_ESCAPE => return Some(UiResult::Cancelled),
        KS_RETURN | KS_KP_ENTER => return Some(state.act(false)),
        _ => {}
    }

    if ctrl {
        match ks {
            KS_C_LOWER => return Some(state.act(true)),
            KS_Z_LOWER => state.canvas.undo(),
            KS_Y_LOWER => state.canvas.redo(),
            _ => {}
        }
        return None;
    }

    if (KS_1..=KS_8).contains(&ks) {
        let idx = (ks - KS_1) as usize;
        if let Some(&t) = ToolKind::ALL.get(idx) {
            state.canvas.tool = Some(t);
        }
    }
    None
}

pub(super) fn pick_cursor(state: &OverlayState, dragging: &Dragging, p: Pos) -> u16 {
    if let Dragging::EditResize = dragging {
        if let SelectionEdit::Resizing(h) = state.selection_edit {
            return cursor_glyph_for_handle(h);
        }
    }
    if let Dragging::EditMove = dragging {
        return XC_FLEUR;
    }
    if state.strip_hover.is_some() {
        return XC_HAND1;
    }
    match state.selection {
        Some(sel) => {
            if let Some(h) = handle_at(sel, p) {
                cursor_glyph_for_handle(h)
            } else if sel.contains(p) {
                if state.ctrl_down {
                    XC_FLEUR
                } else {
                    XC_CROSSHAIR
                }
            } else {
                XC_LEFT_PTR
            }
        }
        None => XC_CROSSHAIR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::ToolKind;
    use crate::config::Config;
    use image::RgbaImage;
    use std::sync::Arc;

    fn test_state() -> OverlayState {
        OverlayState::new(
            RgbaImage::new(200, 200),
            String::new(),
            false,
            Arc::new(Config::default()),
        )
    }

    fn annotate(state: &mut OverlayState, sel: Bounds) {
        state.mode = Mode::Annotating;
        state.selection = Some(sel);
    }

    fn sel() -> Bounds {
        Bounds {
            x: 20.0,
            y: 20.0,
            w: 80.0,
            h: 80.0,
        }
    }

    #[test]
    fn press_selecting_starts_region_drag() {
        let mut state = test_state();
        let d = on_press(&mut state, Pos { x: 10.0, y: 10.0 });
        assert_eq!(d, Dragging::Region);
        assert!(state.sel_drag_start.is_some());
        assert!(state.selection.is_none());
    }

    #[test]
    fn region_drag_then_release_commits_if_large_enough() {
        let mut state = test_state();
        let mut dragging = on_press(&mut state, Pos { x: 10.0, y: 10.0 });
        on_motion(&mut state, &mut dragging, Pos { x: 50.0, y: 60.0 });
        let finish = on_release(
            &mut state,
            &mut dragging,
            Pos { x: 50.0, y: 60.0 },
            Pos { x: 10.0, y: 10.0 },
        );
        assert!(finish.is_none());
        assert_eq!(state.mode, Mode::Annotating);
        let s = state.selection.expect("selection");
        assert_eq!(s.w, 40.0);
        assert_eq!(s.h, 50.0);
    }

    #[test]
    fn tiny_region_is_discarded() {
        let mut state = test_state();
        let mut dragging = on_press(&mut state, Pos { x: 10.0, y: 10.0 });
        on_motion(&mut state, &mut dragging, Pos { x: 12.0, y: 12.0 });
        on_release(
            &mut state,
            &mut dragging,
            Pos { x: 12.0, y: 12.0 },
            Pos { x: 10.0, y: 10.0 },
        );
        assert_eq!(state.mode, Mode::SelectingRegion);
        assert!(state.selection.is_none());
    }

    #[test]
    fn counter_click_places_annotation() {
        let mut state = test_state();
        annotate(&mut state, sel());
        state.canvas.tool = Some(ToolKind::Counter);
        let p = Pos { x: 50.0, y: 50.0 };
        let d = on_press(&mut state, p);
        assert_eq!(d, Dragging::None);
        assert_eq!(state.canvas.annotations.len(), 1);
        match &state.canvas.annotations[0] {
            Annotation::Counter { center, number, .. } => {
                assert_eq!(center.x, 50.0);
                assert_eq!(*number, 1);
            }
            other => panic!("expected Counter, got {other:?}"),
        }
    }

    #[test]
    fn pencil_press_starts_draft() {
        let mut state = test_state();
        annotate(&mut state, sel());
        state.canvas.tool = Some(ToolKind::Pencil);
        let d = on_press(&mut state, Pos { x: 40.0, y: 40.0 });
        assert_eq!(d, Dragging::Draft);
        assert!(state.draft.is_some());
    }

    #[test]
    fn ctrl_inside_selection_moves_frame_even_with_tool() {
        let mut state = test_state();
        annotate(&mut state, sel());
        state.canvas.tool = Some(ToolKind::Pencil);
        state.ctrl_down = true;
        let d = on_press(&mut state, Pos { x: 50.0, y: 50.0 });
        assert_eq!(d, Dragging::EditMove);
        assert!(state.draft.is_none());
    }

    #[test]
    fn escape_cancels() {
        let mut state = test_state();
        assert!(matches!(
            handle_key(&mut state, KS_ESCAPE, false),
            Some(UiResult::Cancelled)
        ));
    }

    #[test]
    fn digit_keys_arm_first_eight_tools() {
        let mut state = test_state();
        assert!(handle_key(&mut state, KS_1, false).is_none());
        assert_eq!(state.canvas.tool, Some(ToolKind::ALL[0]));
        assert!(handle_key(&mut state, KS_8, false).is_none());
        assert_eq!(state.canvas.tool, Some(ToolKind::ALL[7]));
    }

    #[test]
    fn ctrl_z_undoes_last_annotation() {
        let mut state = test_state();
        annotate(&mut state, sel());
        state.canvas.tool = Some(ToolKind::Counter);
        on_press(&mut state, Pos { x: 50.0, y: 50.0 });
        assert_eq!(state.canvas.annotations.len(), 1);
        assert!(handle_key(&mut state, KS_Z_LOWER, true).is_none());
        assert!(state.canvas.annotations.is_empty());
    }

    #[test]
    fn pick_cursor_crosshair_with_no_selection() {
        let state = test_state();
        assert_eq!(
            pick_cursor(&state, &Dragging::None, Pos { x: 1.0, y: 1.0 }),
            XC_CROSSHAIR
        );
    }

    #[test]
    fn pick_cursor_fleur_while_moving() {
        let mut state = test_state();
        annotate(&mut state, sel());
        assert_eq!(
            pick_cursor(&state, &Dragging::EditMove, Pos { x: 50.0, y: 50.0 }),
            XC_FLEUR
        );
    }

    #[test]
    fn apply_hit_toggles_active_tool() {
        let mut state = test_state();
        apply_hit(&mut state, Hit::Tool(ToolKind::Line));
        assert_eq!(state.canvas.tool, Some(ToolKind::Line));
        apply_hit(&mut state, Hit::Tool(ToolKind::Line));
        assert_eq!(state.canvas.tool, None);
    }
}
