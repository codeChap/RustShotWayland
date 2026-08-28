//! Wayland layer-shell overlay (wlr-layer-shell, Overlay layer).
//! Fullscreen on one output, exclusive keyboard, SHM blit of RGBA frames.

use crate::error::{Error, Result};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RepeatInfo},
        pointer::{
            CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
        },
        Capability, SeatHandler, SeatState,
    },
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{
        slot::{Buffer, SlotPool},
        Shm, ShmHandler,
    },
};
use std::collections::VecDeque;
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
    Connection, EventQueue, QueueHandle,
};

pub(super) const XC_CROSSHAIR: u16 = 34;
pub(super) const XC_FLEUR: u16 = 52;
pub(super) const XC_HAND1: u16 = 58;
pub(super) const XC_LEFT_PTR: u16 = 68;

pub(super) const KS_ESCAPE: u32 = 0xff1b;
pub(super) const KS_RETURN: u32 = 0xff0d;
pub(super) const KS_KP_ENTER: u32 = 0xff8d;
pub(super) const KS_C_LOWER: u32 = 0x0063;
pub(super) const KS_Z_LOWER: u32 = 0x007a;
pub(super) const KS_Y_LOWER: u32 = 0x0079;
pub(super) const KS_1: u32 = 0x0031;
pub(super) const KS_8: u32 = 0x0038;

/// Linux evdev BTN_LEFT.
const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone)]
pub(super) enum WlEvent {
    Expose,
    KeyPress {
        keysym: u32,
        ctrl: bool,
        shift: bool,
    },
    KeyRelease {
        ctrl: bool,
        shift: bool,
    },
    ButtonPress {
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    },
    ButtonRelease {
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    },
    Motion {
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    },
}

pub(super) struct WlWin {
    conn: Connection,
    event_queue: EventQueue<WinState>,
    state: WinState,
    width: u32,
    height: u32,
}

struct WinState {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    shm: Shm,
    pool: SlotPool,
    layer: Option<LayerSurface>,
    attached_output: Option<wl_output::WlOutput>,
    events: VecDeque<WlEvent>,
    configured: bool,
    closed: bool,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<ThemedPointer>,
    mods: Modifiers,
    cursor: CursorIcon,
    width: u32,
    height: u32,
    buffers: [Option<Buffer>; 2],
    buf_idx: usize,
}

impl WlWin {
    pub fn new(screen_origin: (i32, i32), output_name: &str, width: u16, height: u16) -> Result<Self> {
        let conn = Connection::connect_to_env()
            .map_err(|e| Error::Other(format!("wayland connect: {e}")))?;
        let (globals, mut event_queue) = registry_queue_init(&conn)
            .map_err(|e| Error::Other(format!("wayland registry: {e}")))?;
        let qh = event_queue.handle();

        let compositor = CompositorState::bind(&globals, &qh)
            .map_err(|e| Error::Other(format!("wl_compositor: {e}")))?;
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(|e| {
            Error::Other(format!(
                "layer-shell unavailable: {e} (need Hyprland / wlroots)"
            ))
        })?;
        let shm = Shm::bind(&globals, &qh).map_err(|e| Error::Other(format!("wl_shm: {e}")))?;

        let pool = SlotPool::new((width as usize) * (height as usize) * 4, &shm)
            .map_err(|e| Error::Other(format!("shm pool: {e}")))?;

        let mut state = WinState {
            registry_state: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor,
            shm,
            pool,
            layer: None,
            attached_output: None,
            events: VecDeque::new(),
            configured: false,
            closed: false,
            keyboard: None,
            pointer: None,
            mods: Modifiers::default(),
            cursor: CursorIcon::Crosshair,
            width: width as u32,
            height: height as u32,
            buffers: [None, None],
            buf_idx: 0,
        };

        // Need output events before pinning the layer to the captured monitor.
        for _ in 0..16 {
            event_queue
                .blocking_dispatch(&mut state)
                .map_err(|e| Error::Other(format!("wayland dispatch: {e}")))?;
            if state.output_state.outputs().next().is_some() {
                break;
            }
        }

        let output = pick_output(&state, screen_origin, output_name);
        if output.is_none() {
            tracing::warn!(
                output_name,
                x = screen_origin.0,
                y = screen_origin.1,
                "no matching wl_output; compositor will pick"
            );
        }
        let surface = state.compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Overlay,
            Some("rustshot"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        layer.set_size(width as u32, height as u32);
        layer.commit();
        state.layer = Some(layer);

        // Wait for first configure (and seat capabilities).
        for _ in 0..64 {
            event_queue
                .blocking_dispatch(&mut state)
                .map_err(|e| Error::Other(format!("wayland dispatch: {e}")))?;
            if state.configured {
                break;
            }
        }
        if !state.configured {
            return Err(Error::Other("layer-shell never configured".into()));
        }

        Ok(Self {
            conn,
            event_queue,
            state,
            width: width as u32,
            height: height as u32,
        })
    }

    pub fn blit_rgba(&mut self, rgba: &[u8]) -> Result<()> {
        draw(
            &mut self.state,
            rgba,
            self.width,
            self.height,
            None,
            &self.event_queue.handle(),
        )?;
        self.event_queue
            .flush()
            .map_err(|e| Error::Other(format!("wayland flush: {e}")))?;
        Ok(())
    }

    pub fn blit_rgba_rect(&mut self, rgba: &[u8], x: u32, y: u32, w: u32, h: u32) -> Result<()> {
        draw(
            &mut self.state,
            rgba,
            self.width,
            self.height,
            Some((x, y, w, h)),
            &self.event_queue.handle(),
        )?;
        self.event_queue
            .flush()
            .map_err(|e| Error::Other(format!("wayland flush: {e}")))?;
        Ok(())
    }

    pub fn set_cursor(&mut self, glyph: u16) -> Result<()> {
        self.state.cursor = cursor_icon(glyph);
        apply_cursor(&self.conn, &self.state);
        Ok(())
    }

    pub fn wait_event(&mut self, cancel: &AtomicBool) -> Result<WlEvent> {
        loop {
            if self.state.closed || cancel.load(Ordering::Acquire) {
                return Ok(WlEvent::KeyPress {
                    keysym: KS_ESCAPE,
                    ctrl: false,
                    shift: false,
                });
            }
            self.event_queue
                .dispatch_pending(&mut self.state)
                .map_err(|e| Error::Other(format!("wayland dispatch: {e}")))?;
            if let Some(ev) = self.state.events.pop_front() {
                return Ok(ev);
            }
            if self.state.closed || cancel.load(Ordering::Acquire) {
                return Ok(WlEvent::KeyPress {
                    keysym: KS_ESCAPE,
                    ctrl: false,
                    shift: false,
                });
            }
            self.event_queue
                .flush()
                .map_err(|e| Error::Other(format!("wayland flush: {e}")))?;
            let Some(guard) = self.event_queue.prepare_read() else {
                continue;
            };
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let n = unsafe { libc::poll(&mut pfd, 1, 200) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(Error::Other(format!("wayland poll: {err}")));
            }
            if n == 0 {
                continue;
            }
            if let Err(e) = guard.read() {
                return Err(Error::Other(format!("wayland read: {e}")));
            }
        }
    }

    pub fn poll_event(&mut self) -> Result<Option<WlEvent>> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map_err(|e| Error::Other(format!("wayland dispatch: {e}")))?;
        Ok(self.state.events.pop_front())
    }
}

fn cursor_icon(glyph: u16) -> CursorIcon {
    match glyph {
        XC_CROSSHAIR => CursorIcon::Crosshair,
        XC_FLEUR => CursorIcon::Move,
        XC_HAND1 => CursorIcon::Pointer,
        XC_LEFT_PTR => CursorIcon::Default,
        138 => CursorIcon::NsResize,
        16 => CursorIcon::EwResize,
        136 | 12 => CursorIcon::NeswResize,
        134 | 14 => CursorIcon::NwseResize,
        70 => CursorIcon::WResize,
        _ => CursorIcon::Default,
    }
}

fn apply_cursor(conn: &Connection, state: &WinState) {
    let Some(ptr) = state.pointer.as_ref() else {
        return;
    };
    if let Err(e) = ptr.set_cursor(conn, state.cursor) {
        tracing::debug!("set_cursor: {e}");
    }
}

fn draw(
    state: &mut WinState,
    rgba: &[u8],
    width: u32,
    height: u32,
    dirty: Option<(u32, u32, u32, u32)>,
    qh: &QueueHandle<WinState>,
) -> Result<()> {
    let stride = width as i32 * 4;
    let nbytes = (width as usize) * (height as usize) * 4;
    let idx = state.buf_idx;

    let can_reuse = state.buffers[idx]
        .as_ref()
        .and_then(|b| b.canvas(&mut state.pool))
        .map(|c| c.len() >= nbytes)
        .unwrap_or(false);
    if !can_reuse {
        let (buffer, _) = state
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride,
                wl_shm::Format::Argb8888,
            )
            .map_err(|e| Error::Other(format!("create buffer: {e}")))?;
        state.buffers[idx] = Some(buffer);
    }

    {
        let canvas = state.buffers[idx]
            .as_ref()
            .unwrap()
            .canvas(&mut state.pool)
            .ok_or_else(|| Error::Other("shm canvas busy".into()))?;
        match dirty {
            Some((x, y, w, h)) => {
                crate::capture::swizzle_rb_opaque_rect(rgba, canvas, width, x, y, w, h);
            }
            None => {
                crate::capture::swizzle_rb_opaque(rgba, &mut canvas[..nbytes]);
            }
        }
    }

    let layer = state
        .layer
        .as_ref()
        .ok_or_else(|| Error::Other("layer surface missing".into()))?;
    match dirty {
        Some((x, y, w, h)) => {
            layer
                .wl_surface()
                .damage_buffer(x as i32, y as i32, w as i32, h as i32);
        }
        None => {
            layer
                .wl_surface()
                .damage_buffer(0, 0, width as i32, height as i32);
        }
    }

    layer.wl_surface().frame(qh, layer.wl_surface().clone());
    state.buffers[idx]
        .as_ref()
        .unwrap()
        .attach_to(layer.wl_surface())
        .map_err(|e| Error::Other(format!("attach: {e:?}")))?;
    layer.commit();
    state.buf_idx = 1 - idx;
    Ok(())
}

impl CompositorHandler for WinState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        self.attached_output = Some(output.clone());
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if self.attached_output.as_ref() == Some(output) {
            self.attached_output = None;
        }
    }
}

impl OutputHandler for WinState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        // Fullscreen exclusive / mode changes can destroy the output the
        // overlay is on. Treat that as close so PrintScr is not stuck.
        if self.attached_output.as_ref() == Some(&output) {
            self.closed = true;
        }
    }
}

impl LayerShellHandler for WinState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 != 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 != 0 {
            self.height = configure.new_size.1;
        }
        self.configured = true;
        self.events.push_back(WlEvent::Expose);
    }
}

impl SeatHandler for WinState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            if let Ok(kb) = self.seat_state.get_keyboard(qh, &seat, None) {
                self.keyboard = Some(kb);
            }
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            let surface = self.compositor.create_surface(qh);
            match self.seat_state.get_pointer_with_theme(
                qh,
                &seat,
                self.shm.wl_shm(),
                surface,
                ThemeSpec::System,
            ) {
                Ok(ptr) => self.pointer = Some(ptr),
                Err(e) => tracing::warn!("themed pointer: {e}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            if let Some(kb) = self.keyboard.take() {
                kb.release();
            }
        }
        if capability == Capability::Pointer {
            self.pointer.take();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for WinState {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _: u32,
    ) {
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        self.events.push_back(WlEvent::KeyPress {
            keysym: event.keysym.raw(),
            ctrl: self.mods.ctrl,
            shift: self.mods.shift,
        });
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _event: KeyEvent,
    ) {
        self.events.push_back(WlEvent::KeyRelease {
            ctrl: self.mods.ctrl,
            shift: self.mods.shift,
        });
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _layout: u32,
    ) {
        self.mods = modifiers;
    }

    fn update_repeat_info(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: RepeatInfo,
    ) {
    }
}

impl PointerHandler for WinState {
    fn pointer_frame(
        &mut self,
        conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        use PointerEventKind::*;
        for event in events {
            let Some(layer) = self.layer.as_ref() else {
                continue;
            };
            if &event.surface != layer.wl_surface() {
                continue;
            }
            let x = event.position.0 as f32;
            let y = event.position.1 as f32;
            let ctrl = self.mods.ctrl;
            let shift = self.mods.shift;
            match event.kind {
                Enter { .. } => {
                    apply_cursor(conn, self);
                    self.events.push_back(WlEvent::Motion { x, y, ctrl, shift });
                }
                Leave { .. } => {}
                Motion { .. } => {
                    self.events.push_back(WlEvent::Motion { x, y, ctrl, shift });
                }
                Press { button, .. } if button == BTN_LEFT => {
                    self.events
                        .push_back(WlEvent::ButtonPress { x, y, ctrl, shift });
                }
                Release { button, .. } if button == BTN_LEFT => {
                    self.events
                        .push_back(WlEvent::ButtonRelease { x, y, ctrl, shift });
                }
                Press { .. } | Release { .. } | Axis { .. } => {}
            }
        }
    }
}

impl ShmHandler for WinState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(WinState);
delegate_output!(WinState);
delegate_shm!(WinState);
delegate_seat!(WinState);
delegate_keyboard!(WinState);
delegate_pointer!(WinState);
delegate_layer!(WinState);
delegate_registry!(WinState);

impl ProvidesRegistryState for WinState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

fn pick_output(state: &WinState, origin: (i32, i32), name: &str) -> Option<wl_output::WlOutput> {
    let mut by_origin = None;
    for output in state.output_state.outputs() {
        let Some(info) = state.output_state.info(&output) else {
            continue;
        };
        if !name.is_empty() && info.name.as_deref() == Some(name) {
            return Some(output);
        }
        if info.location == origin {
            by_origin = Some(output);
        }
    }
    by_origin
}
