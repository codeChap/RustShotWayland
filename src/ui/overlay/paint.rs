//! Composite pipeline: start from `base` (captured image + committed pixelates),
//! apply live annotations + UI chrome (dim, selection frame, handles, strip),
//! then blit to the layer-shell surface.
//!
//! Display buffer is an `RgbaImage` throughout; tiny-skia temporarily wraps it
//! via `PixmapMut::from_bytes` for vector work, imageproc takes it directly
//! for text. Keeps the pipeline single-buffer.

use super::draft::Draft;
use super::selection::{handle_corner_positions, HANDLE_VISUAL};
use super::state::OverlayState;
use super::tool_buttons;
use crate::canvas::{render, Bounds, Pos};
use ab_glyph::PxScale;
use image::RgbaImage;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, PixmapMut, Stroke, Transform};

pub(super) fn composite(display: &mut RgbaImage, state: &OverlayState) {
    display.as_mut().copy_from_slice(state.dim_base.as_raw());
    if let Some(sel) = state.selection {
        copy_rect(
            &state.base,
            display,
            sel.x.max(0.0) as u32,
            sel.y.max(0.0) as u32,
            sel.x.max(0.0) as u32,
            sel.y.max(0.0) as u32,
            sel.w.max(0.0) as u32,
            sel.h.max(0.0) as u32,
        );
    }
    render::rasterize_overlays(display, &state.canvas.annotations);
    if let Some(draft) = &state.draft {
        paint_draft(display, draft, state, Pos { x: 0.0, y: 0.0 });
    }
    if let Some(sel) = state.selection {
        paint_selection_frame(display, sel, &state.theme);
        paint_handles(display, sel, &state.theme);
        let strip = tool_buttons::strip_rect(display.width(), display.height(), sel);
        tool_buttons::paint(display, strip, state.canvas.tool, state.strip_hover, &state.theme);
    } else {
        paint_hint_text(display, &state.theme);
    }
}

/// AABB of the empty-state hint block (not the whole screen).
pub(super) fn hint_bounds(sw: u32, sh: u32) -> Bounds {
    let block_h = 52.0 + 26.0 * 9.0;
    let block_w = 780.0_f32.min(sw as f32 - 16.0).max(200.0);
    Bounds::centered(sw as f32 * 0.5, sh as f32 * 0.5, block_w, block_h)
        .pad(12.0)
        .clamp_to(sw as f32, sh as f32)
}

/// AABB of everything drawn besides the dim backdrop.
pub(super) fn chrome_bounds(state: &OverlayState, sw: u32, sh: u32) -> Bounds {
    let Some(sel) = state.selection else {
        return hint_bounds(sw, sh);
    };
    let mut r = sel.pad(HANDLE_VISUAL * 0.5 + 4.0);
    r = r.union(tool_buttons::strip_rect(sw, sh, sel).pad(2.0));
    if let Some(d) = state.draft.as_ref() {
        if let Some(db) = d.bounds() {
            r = r.union(db.pad(24.0));
        }
    }
    r.clamp_to(sw as f32, sh as f32)
}

/// Restore + redraw only `dirty` (2D). Returns the pixel rect actually painted.
pub(super) fn composite_dirty(
    display: &mut RgbaImage,
    state: &OverlayState,
    dirty: Bounds,
    scratch: &mut Vec<u8>,
) -> Option<(u32, u32, u32, u32)> {
    let dw = display.width();
    let dh = display.height();
    let (x, y, rw, rh) = dirty.to_px(dw, dh)?;
    if x == 0 && y == 0 && rw == dw && rh == dh {
        composite(display, state);
        return Some((0, 0, dw, dh));
    }

    let origin = Pos {
        x: x as f32,
        y: y as f32,
    };
    let n = (rw as usize) * (rh as usize) * 4;
    scratch.resize(n, 0);
    let buf = std::mem::take(scratch);
    let mut tile = RgbaImage::from_raw(rw, rh, buf)?;

    copy_rect(&state.dim_base, &mut tile, x, y, 0, 0, rw, rh);
    if let Some(sel) = state.selection {
        if let Some((sx, sy, sw, sh)) = sel.to_px(dw, dh) {
            let ix = sx.max(x);
            let iy = sy.max(y);
            let ir = (sx + sw).min(x + rw);
            let ib = (sy + sh).min(y + rh);
            if ir > ix && ib > iy {
                copy_rect(
                    &state.base,
                    &mut tile,
                    ix,
                    iy,
                    ix - x,
                    iy - y,
                    ir - ix,
                    ib - iy,
                );
            }
        }
    }

    render::rasterize_overlays_at(&mut tile, &state.canvas.annotations, origin);
    if let Some(draft) = &state.draft {
        paint_draft(&mut tile, draft, state, origin);
    }
    if let Some(sel) = state.selection {
        let local = sel.translate(-origin.x, -origin.y);
        paint_selection_frame(&mut tile, local, &state.theme);
        paint_handles(&mut tile, local, &state.theme);
        let strip = tool_buttons::strip_rect(dw, dh, sel).translate(-origin.x, -origin.y);
        tool_buttons::paint(&mut tile, strip, state.canvas.tool, state.strip_hover, &state.theme);
    } else {
        // Hint is in screen space; draw it shifted onto the tile if it overlaps.
        paint_hint_text_at(&mut tile, dw, dh, origin, &state.theme);
    }

    copy_rect(&tile, display, 0, 0, x, y, rw, rh);
    *scratch = tile.into_raw();
    Some((x, y, rw, rh))
}

fn paint_draft(display: &mut RgbaImage, draft: &Draft, state: &OverlayState, origin: Pos) {
    match draft {
        Draft::Pixelate { .. } => {
            if let Some((px, py, ref img)) = state.draft_pixelate_cache {
                image::imageops::replace(
                    display,
                    img,
                    px as i64 - origin.x as i64,
                    py as i64 - origin.y as i64,
                );
            }
        }
        other => {
            if let Some(a) = other.clone().finalize() {
                render::rasterize_overlays_at(display, &[a], origin);
            }
        }
    }
}

fn copy_rect(
    src: &RgbaImage,
    dst: &mut RgbaImage,
    src_x: u32,
    src_y: u32,
    dst_x: u32,
    dst_y: u32,
    w: u32,
    h: u32,
) {
    if w == 0 || h == 0 {
        return;
    }
    let sw = src.width();
    let sh = src.height();
    let dw = dst.width();
    let dh = dst.height();
    let w = w.min(sw.saturating_sub(src_x)).min(dw.saturating_sub(dst_x));
    let h = h.min(sh.saturating_sub(src_y)).min(dh.saturating_sub(dst_y));
    if w == 0 || h == 0 {
        return;
    }
    let row = (w as usize) * 4;
    let sraw = src.as_raw();
    let draw = dst.as_mut();
    for i in 0..h as usize {
        let so = ((src_y as usize + i) * sw as usize + src_x as usize) * 4;
        let doff = ((dst_y as usize + i) * dw as usize + dst_x as usize) * 4;
        draw[doff..doff + row].copy_from_slice(&sraw[so..so + row]);
    }
}

fn paint_selection_frame(display: &mut RgbaImage, sel: Bounds, theme: &crate::theme::Theme) {
    let w = display.width();
    let h = display.height();
    let buf = display.as_mut();
    let Some(mut pm) = PixmapMut::from_bytes(buf, w, h) else { return; };
    let accent = crate::theme::skia(theme.accent);
    stroke_rect_px(&mut pm, sel.x, sel.y, sel.w, sel.h, accent, 2.0);
}

fn paint_handles(display: &mut RgbaImage, sel: Bounds, theme: &crate::theme::Theme) {
    let w = display.width();
    let h = display.height();
    let buf = display.as_mut();
    let Some(mut pm) = PixmapMut::from_bytes(buf, w, h) else { return; };
    let fill = crate::theme::skia(theme.handle_fill());
    let stroke = crate::theme::skia(theme.accent);
    let s = HANDLE_VISUAL;
    for (_, hx, hy) in handle_corner_positions(sel) {
        let rx = hx - s * 0.5;
        let ry = hy - s * 0.5;
        fill_rect_px(&mut pm, rx, ry, s, s, fill);
        stroke_rect_px(&mut pm, rx, ry, s, s, stroke, 2.0);
    }
}

fn paint_hint_text(display: &mut RgbaImage, theme: &crate::theme::Theme) {
    let w = display.width();
    let h = display.height();
    paint_hint_text_at(display, w, h, Pos { x: 0.0, y: 0.0 }, theme);
}

fn paint_hint_text_at(
    display: &mut RgbaImage,
    screen_w: u32,
    screen_h: u32,
    origin: Pos,
    theme: &crate::theme::Theme,
) {
    let w = screen_w as f32;
    let h = screen_h as f32;
    let font = render::font();
    let title_scale = PxScale::from(34.0);
    let body_scale = PxScale::from(18.0);
    let title_h = 52.0_f32;
    let line_h = 26.0_f32;

    let body: &[&str] = &[
        "Drag to select a region",
        "Enter saves the full screen    Esc cancels",
        "",
        "After selecting a region:",
        "1 Pencil    2 Highlighter    3 Line    4 Arrow",
        "5 Rect    6 Ellipse    7 Pixelate    8 Counter",
        "Drag inside to move the frame    Drag a handle to resize",
        "Shift snaps Line + Arrow to 45\u{b0}",
        "Ctrl+Z undo    Ctrl+Y redo    Ctrl+C copy    Enter save",
    ];

    let block_h = title_h + line_h * body.len() as f32;
    let top = (h * 0.5 - block_h * 0.5) as i32 - origin.y as i32;

    let title = "RUST SHOT";
    let title_color = crate::theme::rgba(theme.bright_foreground);
    let body_color = crate::theme::rgba(theme.foreground);
    let (tw, _) = imageproc::drawing::text_size(title_scale, font, title);
    let tx = (w * 0.5 - tw as f32 * 0.5) as i32 - origin.x as i32;
    imageproc::drawing::draw_text_mut(display, title_color, tx, top, title_scale, font, title);

    let body_top = top + title_h as i32;
    for (i, line) in body.iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let (tw, _) = imageproc::drawing::text_size(body_scale, font, line);
        let tx = (w * 0.5 - tw as f32 * 0.5) as i32 - origin.x as i32;
        let ty = body_top + (i as f32 * line_h) as i32;
        imageproc::drawing::draw_text_mut(display, body_color, tx, ty, body_scale, font, line);
    }
}

// --- tiny-skia helpers -------------------------------------------------------

fn fill_rect_px(pm: &mut PixmapMut, x: f32, y: f32, w: f32, h: f32, c: Color) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = false;
        pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_bounds_is_not_the_whole_screen() {
        let b = hint_bounds(3840, 2160);
        assert!(
            b.area() < 3840.0 * 2160.0 * 0.15,
            "hint AABB should be a center block, got {}x{}",
            b.w,
            b.h
        );
        assert!(b.w > 200.0 && b.h > 100.0);
    }
}

fn stroke_rect_px(pm: &mut PixmapMut, x: f32, y: f32, w: f32, h: f32, c: Color, width: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let Some(r) = tiny_skia::Rect::from_xywh(x, y, w, h) else { return; };
    let mut pb = PathBuilder::new();
    pb.push_rect(r);
    if let Some(path) = pb.finish() {
        let mut p = Paint::default();
        p.set_color(c);
        p.anti_alias = true;
        let mut s = Stroke::default();
        s.width = width;
        pm.stroke_path(&path, &p, &s, Transform::identity(), None);
    }
}
