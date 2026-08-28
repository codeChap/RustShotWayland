//! Wayland overlay — wlr-layer-shell surface, tiny-skia composite, wl_shm blit.

mod draft;
mod input;
mod paint;
mod selection;
mod state;
mod tool_buttons;
mod wl_win;

use crate::canvas::{Bounds, Pos};
use crate::config::Config;
use crate::ui::UiResult;
use image::RgbaImage;
use input::{handle_key, on_motion, on_press, on_release, pick_cursor, update_mods, Dragging};
use state::OverlayState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::oneshot;
use wl_win::{WlEvent, WlWin, XC_CROSSHAIR};

pub fn show(
    image: RgbaImage,
    screen_origin: (i32, i32),
    output_name: String,
    save_path: String,
    clipboard: bool,
    config: Arc<Config>,
    cancel: Arc<AtomicBool>,
    result_tx: oneshot::Sender<UiResult>,
) {
    let t0 = std::time::Instant::now();
    let mut session = match OverlaySession::open(
        image,
        screen_origin,
        output_name,
        save_path,
        clipboard,
        config,
        cancel,
        t0,
    ) {
        Ok(s) => s,
        Err(win) => {
            let _ = result_tx.send(UiResult::Cancelled);
            drop(win);
            return;
        }
    };
    let result = session.run();
    let _ = result_tx.send(result);
    tracing::info!(total_ms = t0.elapsed().as_millis() as u64, "overlay closed");
    drop(session);
}

struct OverlaySession {
    win: WlWin,
    state: OverlayState,
    display: RgbaImage,
    scratch: Vec<u8>,
    dragging: Dragging,
    press_pos: Pos,
    last_cursor: u16,
    pending: Option<Repaint>,
    last_motion: Option<Pos>,
    w: u32,
    h: u32,
    cancel: Arc<AtomicBool>,
}

impl OverlaySession {
    fn open(
        image: RgbaImage,
        screen_origin: (i32, i32),
        output_name: String,
        save_path: String,
        clipboard: bool,
        config: Arc<Config>,
        cancel: Arc<AtomicBool>,
        t0: std::time::Instant,
    ) -> Result<Self, Option<WlWin>> {
        // On first-blit failure the window is returned so the caller can send
        // Cancelled before dropping the layer surface.
        let (w, h) = (image.width(), image.height());

        let mut win = match WlWin::new(screen_origin, &output_name, w as u16, h as u16) {
            Ok(w) => w,
            Err(e) => {
                tracing::error!("Wayland overlay window creation failed: {e}");
                return Err(None);
            }
        };

        tracing::info!(
            setup_ms = t0.elapsed().as_millis() as u64,
            w,
            h,
            "overlay window ready"
        );

        let state = OverlayState::new(image, save_path, clipboard, config);
        let nbytes = (w as usize) * (h as usize) * 4;
        let mut display = RgbaImage::from_raw(w, h, crate::capture::alloc_pixels(nbytes))
            .expect("display buffer size matches");

        // First frame before the pointer cursor is applied so pixels are on
        // screen as soon as the layer-shell surface is ready.
        paint::composite(&mut display, &state);
        if let Err(e) = win.blit_rgba(display.as_raw()) {
            tracing::error!("first blit failed: {e}");
            return Err(Some(win));
        }
        tracing::info!(
            first_paint_ms = t0.elapsed().as_millis() as u64,
            "first frame blitted"
        );

        let _ = win.set_cursor(XC_CROSSHAIR);
        tracing::info!(
            ready_ms = t0.elapsed().as_millis() as u64,
            "overlay interactive"
        );

        Ok(Self {
            win,
            state,
            display,
            scratch: Vec::new(),
            dragging: Dragging::None,
            press_pos: Pos { x: 0.0, y: 0.0 },
            last_cursor: XC_CROSSHAIR,
            pending: None,
            last_motion: None,
            w,
            h,
            cancel,
        })
    }

    fn run(&mut self) -> UiResult {
        loop {
            if self.cancel.load(Ordering::Acquire) {
                tracing::info!("overlay cancelled by a later capture");
                break UiResult::Cancelled;
            }
            if let Err(result) = self.flush_pending() {
                break result;
            }
            let events = match self.drain_events() {
                Ok(evs) => evs,
                Err(result) => break result,
            };
            if let Some(result) = self.dispatch(events) {
                break result;
            }
        }
    }

    fn flush_pending(&mut self) -> Result<(), UiResult> {
        let Some(repaint) = self.pending.take() else {
            return Ok(());
        };
        let before = paint::chrome_bounds(&self.state, self.w, self.h);
        if let Some(p) = self.last_motion.take() {
            on_motion(&mut self.state, &mut self.dragging, p);
            let desired = pick_cursor(&self.state, &self.dragging, p);
            if desired != self.last_cursor {
                let _ = self.win.set_cursor(desired);
                self.last_cursor = desired;
            }
        }
        let after = paint::chrome_bounds(&self.state, self.w, self.h);
        let _ = self.state.refresh_base();
        self.state.refresh_draft_pixelate();

        let mut kind = repaint;
        kind = merge_repaint(kind, before, self.w, self.h);
        kind = merge_repaint(kind, after, self.w, self.h);

        let t_frame = std::time::Instant::now();
        match kind {
            Repaint::Full => {
                paint::composite(&mut self.display, &self.state);
                if let Err(e) = self.win.blit_rgba(self.display.as_raw()) {
                    tracing::error!("blit failed: {e}");
                    return Err(UiResult::Cancelled);
                }
            }
            Repaint::Rect(r) => {
                if let Some((x, y, rw, rh)) =
                    paint::composite_dirty(&mut self.display, &self.state, r, &mut self.scratch)
                {
                    if let Err(e) = self.win.blit_rgba_rect(self.display.as_raw(), x, y, rw, rh) {
                        tracing::error!("blit rect failed: {e}");
                        return Err(UiResult::Cancelled);
                    }
                }
            }
        }
        tracing::debug!(
            frame_us = t_frame.elapsed().as_micros() as u64,
            ?kind,
            "frame"
        );
        Ok(())
    }

    fn drain_events(&mut self) -> Result<Vec<WlEvent>, UiResult> {
        // Wait for the next event, then drain the queue before painting again.
        // This collapses a burst of MotionNotify into one repaint.
        let first = match self.win.wait_event(&self.cancel) {
            Ok(ev) => ev,
            Err(e) => {
                tracing::error!("wayland wait_for_event: {e}");
                return Err(UiResult::Cancelled);
            }
        };
        let mut events: Vec<WlEvent> = Vec::with_capacity(8);
        events.push(first);
        loop {
            match self.win.poll_event() {
                Ok(Some(ev)) => events.push(ev),
                Ok(None) => break,
                Err(e) => {
                    tracing::error!("wayland poll_for_event: {e}");
                    break;
                }
            }
        }
        Ok(events)
    }

    fn dispatch(&mut self, events: Vec<WlEvent>) -> Option<UiResult> {
        for ev in events {
            match ev {
                WlEvent::Expose => self.pending = Some(Repaint::Full),
                WlEvent::KeyPress {
                    keysym,
                    ctrl,
                    shift,
                } => {
                    if let Some(res) = self.with_chrome_repaint(|s| {
                        update_mods(&mut s.state, ctrl, shift);
                        handle_key(&mut s.state, keysym, ctrl)
                    }) {
                        return Some(res);
                    }
                }
                WlEvent::KeyRelease { ctrl, shift } => {
                    update_mods(&mut self.state, ctrl, shift);
                }
                WlEvent::ButtonPress { x, y, ctrl, shift } => {
                    self.with_chrome_repaint(|s| {
                        s.apply_deferred_motion();
                        let p = Pos { x, y };
                        s.press_pos = p;
                        update_mods(&mut s.state, ctrl, shift);
                        s.dragging = on_press(&mut s.state, p);
                        None
                    });
                }
                WlEvent::Motion { x, y, ctrl, shift } => {
                    let p = Pos { x, y };
                    update_mods(&mut self.state, ctrl, shift);
                    self.last_motion = Some(p);
                    if self.pending.is_none() {
                        self.pending = Some(rect_repaint(&self.state, self.w, self.h));
                    }
                }
                WlEvent::ButtonRelease { x, y, ctrl, shift } => {
                    if let Some(res) = self.with_chrome_repaint(|s| {
                        s.apply_deferred_motion();
                        let p = Pos { x, y };
                        update_mods(&mut s.state, ctrl, shift);
                        on_release(&mut s.state, &mut s.dragging, p, s.press_pos)
                    }) {
                        return Some(res);
                    }
                }
            }
        }
        None
    }

    fn apply_deferred_motion(&mut self) {
        if let Some(p) = self.last_motion.take() {
            on_motion(&mut self.state, &mut self.dragging, p);
        }
    }

    fn with_chrome_repaint(
        &mut self,
        f: impl FnOnce(&mut Self) -> Option<UiResult>,
    ) -> Option<UiResult> {
        let before = paint::chrome_bounds(&self.state, self.w, self.h);
        let res = f(self);
        if res.is_none() {
            acc_repaint(&mut self.pending, before, self.w, self.h);
            acc_repaint(
                &mut self.pending,
                paint::chrome_bounds(&self.state, self.w, self.h),
                self.w,
                self.h,
            );
        }
        res
    }
}

#[derive(Debug, Clone, Copy)]
enum Repaint {
    Full,
    Rect(Bounds),
}

const EMPTY_BOUNDS: Bounds = Bounds {
    x: 0.0,
    y: 0.0,
    w: 0.0,
    h: 0.0,
};

fn rect_repaint(state: &OverlayState, sw: u32, sh: u32) -> Repaint {
    merge_repaint(
        Repaint::Rect(EMPTY_BOUNDS),
        paint::chrome_bounds(state, sw, sh),
        sw,
        sh,
    )
}

fn acc_repaint(pending: &mut Option<Repaint>, bounds: Bounds, sw: u32, sh: u32) {
    let next = match *pending {
        None => merge_repaint(Repaint::Rect(EMPTY_BOUNDS), bounds, sw, sh),
        Some(kind) => merge_repaint(kind, bounds, sw, sh),
    };
    *pending = Some(next);
}

fn merge_repaint(kind: Repaint, bounds: Bounds, sw: u32, sh: u32) -> Repaint {
    if matches!(kind, Repaint::Full) {
        return Repaint::Full;
    }
    let b = bounds.clamp_to(sw as f32, sh as f32);
    if b.w < 1.0 || b.h < 1.0 {
        return kind;
    }
    let u = match kind {
        Repaint::Full => return Repaint::Full,
        Repaint::Rect(r) if r.w < 1.0 || r.h < 1.0 => b,
        Repaint::Rect(r) => r.union(b),
    };
    if u.area() >= (sw as f32) * (sh as f32) * 0.75 {
        Repaint::Full
    } else {
        Repaint::Rect(u)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    #[test]
    fn merge_repaint_full_stays_full() {
        let r = merge_repaint(Repaint::Full, rect(0.0, 0.0, 10.0, 10.0), 100, 100);
        assert!(matches!(r, Repaint::Full));
    }

    #[test]
    fn merge_repaint_ignores_empty_bounds() {
        let r = merge_repaint(
            Repaint::Rect(rect(5.0, 5.0, 10.0, 10.0)),
            EMPTY_BOUNDS,
            100,
            100,
        );
        match r {
            Repaint::Rect(b) => {
                assert_eq!(b.x, 5.0);
                assert_eq!(b.w, 10.0);
            }
            Repaint::Full => panic!("expected rect"),
        }
    }

    #[test]
    fn merge_repaint_unions_rects() {
        let r = merge_repaint(
            Repaint::Rect(rect(0.0, 0.0, 10.0, 10.0)),
            rect(20.0, 20.0, 10.0, 10.0),
            100,
            100,
        );
        match r {
            Repaint::Rect(b) => {
                assert_eq!(b.x, 0.0);
                assert_eq!(b.y, 0.0);
                assert_eq!(b.w, 30.0);
                assert_eq!(b.h, 30.0);
            }
            Repaint::Full => panic!("union of two small rects should stay dirty"),
        }
    }

    #[test]
    fn merge_repaint_promotes_large_union_to_full() {
        let r = merge_repaint(
            Repaint::Rect(rect(0.0, 0.0, 80.0, 80.0)),
            rect(20.0, 20.0, 80.0, 80.0),
            100,
            100,
        );
        assert!(matches!(r, Repaint::Full));
    }

    #[test]
    fn acc_repaint_from_none_uses_bounds() {
        let mut pending = None;
        acc_repaint(&mut pending, rect(2.0, 3.0, 8.0, 9.0), 100, 100);
        match pending {
            Some(Repaint::Rect(b)) => {
                assert_eq!(b.x, 2.0);
                assert_eq!(b.y, 3.0);
                assert_eq!(b.w, 8.0);
                assert_eq!(b.h, 9.0);
            }
            other => panic!("expected rect, got {other:?}"),
        }
    }
}
