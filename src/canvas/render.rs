use crate::canvas::{widget, Annotation, Bounds, Pos, WidgetKind};
use ab_glyph::{Font, FontRef};
use image::{Rgba, RgbaImage};
use tiny_skia::{
    Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, PixmapMut, Rect, Stroke, Transform,
};

const FONT_BYTES: &[u8] = include_bytes!("../../assets/font.ttf");

pub(crate) fn font() -> &'static FontRef<'static> {
    static FONT: std::sync::OnceLock<FontRef<'static>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        FontRef::try_from_slice(FONT_BYTES).expect("embedded font is valid TTF")
    })
}

/// Pass 2 + 3: vector primitives via tiny-skia, then counter text via imageproc.
/// Pixelate annotations are skipped — caller is responsible for having baked
/// them in already (or wanting them omitted, e.g. when `img` is the cached
/// committed_base).
pub fn rasterize_overlays(img: &mut RgbaImage, annotations: &[Annotation]) {
    rasterize_overlays_at(img, annotations, Pos { x: 0.0, y: 0.0 });
}

/// Rasterize into `img`, treating `img`'s (0,0) as screen-space `origin`.
/// Tiny-skia clips to `img`; that's the 2D dirty-rect path.
pub fn rasterize_overlays_at(
    img: &mut RgbaImage,
    annotations: &[Annotation],
    origin: Pos,
) {
    if annotations.is_empty() {
        return;
    }
    let w = img.width();
    let h = img.height();
    {
        let buf = img.as_mut();
        let mut pixmap = match PixmapMut::from_bytes(buf, w, h) {
            Some(p) => p,
            None => {
                tracing::error!("PixmapMut::from_bytes failed (w={w}, h={h})");
                return;
            }
        };
        for a in annotations {
            match a {
                Annotation::Pixelate { .. } | Annotation::Stamp { .. } => {}
                Annotation::Pencil { points, color, width } => {
                    draw_polyline(&mut pixmap, points, *color, *width, origin);
                }
                Annotation::Line { start, end, color, width } => {
                    draw_line(
                        &mut pixmap,
                        shift(*start, origin),
                        shift(*end, origin),
                        *color,
                        *width,
                    );
                }
                Annotation::Arrow { start, end, color, width } => {
                    draw_arrow(
                        &mut pixmap,
                        shift(*start, origin),
                        shift(*end, origin),
                        *color,
                        *width,
                    );
                }
                Annotation::Rect { rect, color, width } => {
                    draw_rect_outline(&mut pixmap, shift_bounds(*rect, origin), *color, *width);
                }
                Annotation::Ellipse { rect, color, width } => {
                    draw_ellipse_outline(&mut pixmap, shift_bounds(*rect, origin), *color, *width);
                }
                Annotation::Counter { center, color, radius, .. } => {
                    draw_counter_circle(&mut pixmap, shift(*center, origin), *color, *radius);
                }
                Annotation::Widget { kind, rect, color, width } => {
                    draw_widget(
                        &mut pixmap,
                        *kind,
                        shift_bounds(*rect, origin),
                        *color,
                        *width,
                    );
                }
            }
        }
    }

    let font = font();
    for a in annotations {
        match a {
            Annotation::Counter { center, number, color, radius } => {
                draw_counter_text(img, shift(*center, origin), *number, *color, *radius, font);
            }
            Annotation::Stamp { center, ch, color, size } => {
                draw_stamp_text(img, shift(*center, origin), *ch, *color, *size, font);
            }
            Annotation::Widget {
                kind: WidgetKind::Measure,
                rect,
                color,
                ..
            } => {
                draw_measure_label(img, shift_bounds(*rect, origin), *color, font);
            }
            _ => {}
        }
    }
}

fn shift(p: Pos, origin: Pos) -> Pos {
    Pos {
        x: p.x - origin.x,
        y: p.y - origin.y,
    }
}

fn shift_bounds(b: Bounds, origin: Pos) -> Bounds {
    Bounds {
        x: b.x - origin.x,
        y: b.y - origin.y,
        w: b.w,
        h: b.h,
    }
}

fn paint_for(color: Rgba<u8>) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(color.0[0], color.0[1], color.0[2], color.0[3]));
    p.anti_alias = true;
    p
}

fn stroke_for(width: f32) -> Stroke {
    let mut s = Stroke::default();
    s.width = width.max(0.5);
    s.line_cap = LineCap::Round;
    s.line_join = LineJoin::Round;
    s
}

fn fill(pixmap: &mut PixmapMut, path: &tiny_skia::Path, color: Rgba<u8>) {
    pixmap.fill_path(
        path,
        &paint_for(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn stroke(pixmap: &mut PixmapMut, path: &tiny_skia::Path, color: Rgba<u8>, width: f32) {
    pixmap.stroke_path(
        path,
        &paint_for(color),
        &stroke_for(width),
        Transform::identity(),
        None,
    );
}

fn draw_polyline(pixmap: &mut PixmapMut, points: &[Pos], color: Rgba<u8>, width: f32, origin: Pos) {
    if points.len() < 2 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].x - origin.x, points[0].y - origin.y);
    for p in &points[1..] {
        pb.line_to(p.x - origin.x, p.y - origin.y);
    }
    if let Some(path) = pb.finish() {
        stroke(pixmap, &path, color, width);
    }
}

fn draw_line(pixmap: &mut PixmapMut, start: Pos, end: Pos, color: Rgba<u8>, width: f32) {
    let mut pb = PathBuilder::new();
    pb.move_to(start.x, start.y);
    pb.line_to(end.x, end.y);
    if let Some(path) = pb.finish() {
        stroke(pixmap, &path, color, width);
    }
}

fn draw_arrow(pixmap: &mut PixmapMut, start: Pos, end: Pos, color: Rgba<u8>, width: f32) {
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let head = (width * 4.0).max(12.0);
    let ux = dx / len;
    let uy = dy / len;
    let angle = 28f32.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    // Stop the shaft at the head's base, not the tip. `stroke_for` uses a round
    // cap, so a shaft drawn all the way to `end` bulges a half-round blob out
    // past the point — it reads as a dot stuck on the end of the arrow.
    // The base sits exactly `head * cos_a` back along the axis, so the cap ends
    // up inside the triangle and the two shapes meet with no seam.
    let backoff = head * cos_a;
    if len > backoff {
        let mut pb = PathBuilder::new();
        pb.move_to(start.x, start.y);
        pb.line_to(end.x - ux * backoff, end.y - uy * backoff);
        if let Some(path) = pb.finish() {
            stroke(pixmap, &path, color, width);
        }
    }

    let h1 = (
        end.x - head * (ux * cos_a - uy * sin_a),
        end.y - head * (uy * cos_a + ux * sin_a),
    );
    let h2 = (
        end.x - head * (ux * cos_a + uy * sin_a),
        end.y - head * (uy * cos_a - ux * sin_a),
    );
    let mut pb = PathBuilder::new();
    pb.move_to(h1.0, h1.1);
    pb.line_to(end.x, end.y);
    pb.line_to(h2.0, h2.1);
    pb.close();
    if let Some(path) = pb.finish() {
        fill(pixmap, &path, color);
    }
}

/// Vector-only widget stamp. Used by the overlay compositor and the tool-strip
/// glyphs so both stay in lockstep. tiny-skia 0.11 has no `push_rounded_rect`.
pub(crate) fn draw_widget(
    pixmap: &mut PixmapMut,
    kind: WidgetKind,
    rect: Bounds,
    color: Rgba<u8>,
    width: f32,
) {
    match kind {
        WidgetKind::Button => fill_rounded(pixmap, rect, widget::corner_radius(rect), color),
        WidgetKind::Input => {
            stroke_rounded(pixmap, rect, widget::corner_radius(rect), color, width);
            fill_bounds(pixmap, widget::input_placeholder(rect), color);
        }
        WidgetKind::ImageX => {
            draw_rect_outline(pixmap, rect, color, width);
            draw_line(pixmap, rect.nw(), rect.se(), color, width);
            draw_line(pixmap, rect.ne(), rect.sw(), color, width);
        }
        WidgetKind::Checkbox => {
            let sq = widget::checkbox_square(rect);
            stroke_rounded(pixmap, sq, 3.0_f32.min(sq.w * 0.12), color, width);
            let (a, m, c) = widget::checkbox_tick(sq);
            draw_line(pixmap, a, m, color, width);
            draw_line(pixmap, m, c, color, width);
        }
        WidgetKind::Toggle => {
            stroke_rounded(pixmap, rect, rect.h * 0.5, color, width);
            let (center, r) = widget::toggle_knob(rect);
            fill_circle(pixmap, center, r, color);
        }
        WidgetKind::Measure => {
            draw_rect_outline(pixmap, rect, color, width);
            let tick = (width * 2.5).max(4.0).min(rect.w.min(rect.h) * 0.2);
            let c = rect.center();
            draw_line(
                pixmap,
                Pos { x: c.x, y: rect.y },
                Pos { x: c.x, y: rect.y + tick },
                color,
                width,
            );
            draw_line(
                pixmap,
                Pos { x: rect.right(), y: c.y },
                Pos {
                    x: rect.right() - tick,
                    y: c.y,
                },
                color,
                width,
            );
        }
    }
}

fn fill_circle(pixmap: &mut PixmapMut, c: Pos, r: f32, color: Rgba<u8>) {
    if r <= 0.0 {
        return;
    }
    let mut pb = PathBuilder::new();
    pb.push_circle(c.x, c.y, r);
    if let Some(path) = pb.finish() {
        fill(pixmap, &path, color);
    }
}

fn fill_rounded(pixmap: &mut PixmapMut, b: Bounds, radius: f32, color: Rgba<u8>) {
    if let Some(path) = rounded_path(b, radius) {
        fill(pixmap, &path, color);
    }
}

fn stroke_rounded(pixmap: &mut PixmapMut, b: Bounds, radius: f32, color: Rgba<u8>, width: f32) {
    if let Some(path) = rounded_path(b, radius) {
        stroke(pixmap, &path, color, width);
    }
}

fn fill_bounds(pixmap: &mut PixmapMut, b: Bounds, color: Rgba<u8>) {
    if let Some(path) = rect_path(b) {
        fill(pixmap, &path, color);
    }
}

fn rect_path(b: Bounds) -> Option<tiny_skia::Path> {
    if b.w <= 0.0 || b.h <= 0.0 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.push_rect(Rect::from_xywh(b.x, b.y, b.w, b.h)?);
    pb.finish()
}

/// Cubic-approx rounded rect. Falls back to a sharp rect when `radius` is tiny.
fn rounded_path(b: Bounds, radius: f32) -> Option<tiny_skia::Path> {
    if b.w <= 0.0 || b.h <= 0.0 {
        return None;
    }
    let r = radius.min(b.w * 0.5).min(b.h * 0.5).max(0.0);
    let mut pb = PathBuilder::new();
    if r < 0.5 {
        pb.push_rect(Rect::from_xywh(b.x, b.y, b.w, b.h)?);
    } else {
        // 4/3 * tan(pi/8) ≈ 0.55228475 — one cubic per quarter-circle.
        const KAPPA: f32 = 0.552_284_8;
        let k = KAPPA * r;
        let x0 = b.x;
        let y0 = b.y;
        let x1 = b.right();
        let y1 = b.bottom();
        pb.move_to(x0 + r, y0);
        pb.line_to(x1 - r, y0);
        pb.cubic_to(x1 - r + k, y0, x1, y0 + r - k, x1, y0 + r);
        pb.line_to(x1, y1 - r);
        pb.cubic_to(x1, y1 - r + k, x1 - r + k, y1, x1 - r, y1);
        pb.line_to(x0 + r, y1);
        pb.cubic_to(x0 + r - k, y1, x0, y1 - r + k, x0, y1 - r);
        pb.line_to(x0, y0 + r);
        pb.cubic_to(x0, y0 + r - k, x0 + r - k, y0, x0 + r, y0);
        pb.close();
    }
    pb.finish()
}

fn draw_rect_outline(pixmap: &mut PixmapMut, b: Bounds, color: Rgba<u8>, width: f32) {
    if let Some(path) = rect_path(b) {
        stroke(pixmap, &path, color, width);
    }
}

fn draw_ellipse_outline(pixmap: &mut PixmapMut, b: Bounds, color: Rgba<u8>, width: f32) {
    if b.w <= 0.0 || b.h <= 0.0 {
        return;
    }
    let r = match Rect::from_xywh(b.x, b.y, b.w, b.h) {
        Some(r) => r,
        None => return,
    };
    let mut pb = PathBuilder::new();
    pb.push_oval(r);
    if let Some(path) = pb.finish() {
        stroke(pixmap, &path, color, width);
    }
}

fn draw_counter_circle(pixmap: &mut PixmapMut, center: Pos, color: Rgba<u8>, radius: f32) {
    let mut pb = PathBuilder::new();
    pb.push_circle(center.x, center.y, radius);
    let path = match pb.finish() {
        Some(p) => p,
        None => return,
    };
    fill(pixmap, &path, Rgba([255, 255, 255, 255]));
    stroke(pixmap, &path, color, 2.5);
}

fn draw_centered_text(
    img: &mut RgbaImage,
    center: Pos,
    text: &str,
    color: Rgba<u8>,
    scale: ab_glyph::PxScale,
    font: &impl Font,
) {
    let (tw, th) = imageproc::drawing::text_size(scale, font, text);
    imageproc::drawing::draw_text_mut(
        img,
        color,
        center.x as i32 - tw as i32 / 2,
        center.y as i32 - th as i32 / 2,
        scale,
        font,
        text,
    );
}

fn draw_counter_text(
    img: &mut RgbaImage,
    center: Pos,
    number: u32,
    color: Rgba<u8>,
    radius: f32,
    font: &impl Font,
) {
    draw_centered_text(
        img,
        center,
        &number.to_string(),
        color,
        ab_glyph::PxScale::from(radius * 1.2),
        font,
    );
}

fn draw_measure_label(img: &mut RgbaImage, rect: Bounds, color: Rgba<u8>, font: &impl Font) {
    if rect.w < 36.0 || rect.h < 18.0 {
        return;
    }
    let (wp, hp) = widget::measure_px(rect);
    let scale = ab_glyph::PxScale::from((rect.h * 0.32).clamp(10.0, 20.0));
    let ws = wp.to_string();
    let hs = hp.to_string();
    let (tw, th) = imageproc::drawing::text_size(scale, font, &ws);
    let (hw, hh) = imageproc::drawing::text_size(scale, font, &hs);
    let cross = (th as f32 * 0.45).max(4.0);
    let gap = cross + 6.0;
    let total = tw as f32 + gap + hw as f32;
    if total + 4.0 > rect.w {
        return;
    }
    let x0 = rect.x + (rect.w - total) * 0.5;
    let y = rect.y + (rect.h - th.max(hh) as f32) * 0.5;
    imageproc::drawing::draw_text_mut(img, color, x0 as i32, y as i32, scale, font, &ws);
    let cx = x0 + tw as f32 + gap * 0.5;
    let cy = y + th as f32 * 0.45;
    draw_times_cross(img, cx, cy, cross * 0.5, color);
    imageproc::drawing::draw_text_mut(
        img,
        color,
        (x0 + tw as f32 + gap) as i32,
        y as i32,
        scale,
        font,
        &hs,
    );
}

fn draw_times_cross(img: &mut RgbaImage, cx: f32, cy: f32, half: f32, color: Rgba<u8>) {
    imageproc::drawing::draw_line_segment_mut(
        img,
        (cx - half, cy - half),
        (cx + half, cy + half),
        color,
    );
    imageproc::drawing::draw_line_segment_mut(
        img,
        (cx + half, cy - half),
        (cx - half, cy + half),
        color,
    );
}

fn draw_stamp_text(
    img: &mut RgbaImage,
    center: Pos,
    ch: char,
    color: Rgba<u8>,
    size: f32,
    font: &impl Font,
) {
    draw_centered_text(
        img,
        center,
        &ch.to_string(),
        color,
        ab_glyph::PxScale::from(size),
        font,
    );
}

/// Crop + pixelate a region via downscale→upscale (nearest). Returns the clamped
/// origin and the pixelated image so callers can paste it back into the base
/// (committed) or use it as a live preview (draft).
pub fn pixelate_crop(img: &RgbaImage, b: Bounds, block: u32) -> Option<(u32, u32, RgbaImage)> {
    let img_w = img.width();
    let img_h = img.height();
    let x = b.x.max(0.0) as u32;
    let y = b.y.max(0.0) as u32;
    let w = (b.w.max(0.0) as u32).min(img_w.saturating_sub(x));
    let h = (b.h.max(0.0) as u32).min(img_h.saturating_sub(y));
    if w == 0 || h == 0 {
        return None;
    }
    let block = block.max(2);
    let sw = (w / block).max(1);
    let sh = (h / block).max(1);
    let cropped = image::imageops::crop_imm(img, x, y, w, h).to_image();
    // Triangle downscale = area-averaged blocks; Nearest upscale keeps hard edges.
    let down = image::imageops::resize(&cropped, sw, sh, image::imageops::FilterType::Triangle);
    let up = image::imageops::resize(&down, w, h, image::imageops::FilterType::Nearest);
    Some((x, y, up))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid_image(w: u32, h: u32, color: Rgba<u8>) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for p in img.pixels_mut() {
            *p = color;
        }
        img
    }

    #[test]
    fn pixelate_crop_basic() {
        let red = Rgba([255, 0, 0, 255]);
        let img = solid_image(100, 100, red);

        let b = Bounds {
            x: 10.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        };
        let res = pixelate_crop(&img, b, 4);
        assert!(res.is_some());
        let (x, y, out) = res.unwrap();
        assert_eq!(x, 10);
        assert_eq!(y, 10);
        assert_eq!(out.width(), 20);
        assert_eq!(out.height(), 20);
        // After nearest-neighbor upscale from 5x5, every pixel should still be red
        assert_eq!(out.get_pixel(0, 0), &red);
    }

    #[test]
    fn pixelate_crop_clamps_to_image_bounds() {
        let img = solid_image(50, 50, Rgba([0, 255, 0, 255]));
        let b = Bounds {
            x: 40.0,
            y: 40.0,
            w: 30.0,
            h: 30.0,
        };
        let res = pixelate_crop(&img, b, 5);
        assert!(res.is_some());
        let (_x, _y, out) = res.unwrap();
        assert_eq!(out.width(), 10); // 50-40 = 10
        assert_eq!(out.height(), 10);
    }

    #[test]
    fn pixelate_crop_zero_or_negative_area_returns_none() {
        let img = solid_image(100, 100, Rgba([0, 0, 255, 255]));
        let b = Bounds {
            x: 50.0,
            y: 50.0,
            w: 0.0,
            h: 5.0,
        };
        assert!(pixelate_crop(&img, b, 2).is_none());
    }

    #[test]
    fn pixelate_crop_block_min_2() {
        let img = solid_image(16, 16, Rgba([255, 255, 0, 255]));
        let b = Bounds {
            x: 0.0,
            y: 0.0,
            w: 8.0,
            h: 8.0,
        };
        let (_x, _y, out) = pixelate_crop(&img, b, 1).unwrap(); // should force block>=2
        assert_eq!(out.width(), 8);
    }

    #[test]
    fn rasterize_overlays_at_origin_clips() {
        let bg = Rgba([0, 0, 0, 255]);
        let mut img = solid_image(40, 20, bg);
        rasterize_overlays_at(
            &mut img,
            &[Annotation::Line {
                start: Pos { x: 5.0, y: 50.0 },
                end: Pos { x: 35.0, y: 50.0 },
                color: Rgba([255, 50, 50, 255]),
                width: 4.0,
            }],
            Pos { x: 0.0, y: 40.0 },
        );
        assert_ne!(
            img.get_pixel(20, 10),
            &bg,
            "line at screen y=50 is scratch y=10"
        );
    }

    #[test]
    fn rasterize_overlays_empty_is_noop() {
        let mut img = solid_image(64, 64, Rgba([10, 20, 30, 255]));
        let original = img.clone();
        rasterize_overlays(&mut img, &[]);
        assert_eq!(img.as_raw(), original.as_raw());
    }

    #[test]
    fn rasterize_overlays_does_not_panic_on_various_annotations() {
        let mut img = RgbaImage::new(200, 150);
        // Fill with something
        for p in img.pixels_mut() {
            *p = Rgba([100, 100, 100, 255]);
        }

        let anns: Vec<Annotation> = vec![
            Annotation::Line {
                start: Pos { x: 10.0, y: 10.0 },
                end: Pos { x: 50.0, y: 50.0 },
                color: Rgba([255, 0, 0, 255]),
                width: 3.0,
            },
            Annotation::Rect {
                rect: Bounds {
                    x: 20.0,
                    y: 20.0,
                    w: 40.0,
                    h: 30.0,
                },
                color: Rgba([0, 255, 0, 255]),
                width: 2.0,
            },
            Annotation::Counter {
                center: Pos { x: 80.0, y: 80.0 },
                number: 7,
                color: Rgba([0, 0, 255, 255]),
                radius: 14.0,
            },
            Annotation::Stamp {
                center: Pos { x: 120.0, y: 60.0 },
                ch: '!',
                color: Rgba([255, 255, 0, 255]),
                size: 18.0,
            },
            Annotation::Widget {
                kind: WidgetKind::Button,
                rect: Bounds {
                    x: 10.0,
                    y: 100.0,
                    w: 50.0,
                    h: 20.0,
                },
                color: Rgba([255, 50, 50, 255]),
                width: 4.0,
            },
        ];

        // Should not panic and should modify the image
        rasterize_overlays(&mut img, &anns);
        // At least some pixels should have changed (very loose check)
        let changed = img.pixels().any(|p| p.0 != [100, 100, 100, 255]);
        assert!(changed, "rasterize should have drawn something");
    }

    #[test]
    fn rasterize_widget_button_paints_fill() {
        let bg = Rgba([0, 0, 0, 255]);
        let mut img = solid_image(80, 40, bg);
        rasterize_overlays(
            &mut img,
            &[Annotation::Widget {
                kind: WidgetKind::Button,
                rect: Bounds {
                    x: 10.0,
                    y: 8.0,
                    w: 60.0,
                    h: 24.0,
                },
                color: Rgba([255, 50, 50, 255]),
                width: 4.0,
            }],
        );
        // Centre of a filled button must not stay background.
        assert_ne!(img.get_pixel(40, 20), &bg);
    }

    fn widget(kind: WidgetKind, x: f32, y: f32, w: f32, h: f32) -> Annotation {
        Annotation::Widget {
            kind,
            rect: Bounds { x, y, w, h },
            color: Rgba([255, 50, 50, 255]),
            width: 4.0,
        }
    }

    #[test]
    fn rasterize_checkbox_and_toggle_paint() {
        let bg = Rgba([0, 0, 0, 255]);
        let mut img = solid_image(80, 50, bg);
        rasterize_overlays(&mut img, &[widget(WidgetKind::Checkbox, 8.0, 8.0, 36.0, 36.0)]);
        assert!(img.pixels().any(|p| p.0 != bg.0), "checkbox should ink");

        let mut img = solid_image(100, 40, bg);
        rasterize_overlays(&mut img, &[widget(WidgetKind::Toggle, 10.0, 8.0, 80.0, 24.0)]);
        let (knob, _) = widget::toggle_knob(Bounds {
            x: 10.0,
            y: 8.0,
            w: 80.0,
            h: 24.0,
        });
        assert_ne!(
            img.get_pixel(knob.x as u32, knob.y as u32),
            &bg,
            "toggle knob should be filled"
        );
    }

    #[test]
    fn rasterize_measure_paints_outline_and_digits() {
        let bg = Rgba([0, 0, 0, 255]);
        let mut img = solid_image(160, 60, bg);
        rasterize_overlays(&mut img, &[widget(WidgetKind::Measure, 10.0, 10.0, 140.0, 40.0)]);
        assert!(img.pixels().any(|p| p.0 != bg.0), "measure should ink");
    }
}

