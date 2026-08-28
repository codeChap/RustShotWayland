//! In-process wlr-screencopy via libwayshot, with `grim` as fallback.

use super::Screen;
use crate::error::{Error, Result};
use image::{ImageReader, RgbaImage};
use libwayshot::WayshotConnection;
use std::io::Cursor;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

const WAYSHOT_TIMEOUT: Duration = Duration::from_millis(1500);
const GRIM_TIMEOUT: Duration = Duration::from_secs(3);

pub struct WaylandCapture {
    wayshot: Mutex<Option<WayshotConnection>>,
    cached_screens: Mutex<Vec<Screen>>,
}

impl WaylandCapture {
    pub fn new() -> Result<Self> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(Error::Other(
                "WAYLAND_DISPLAY is not set — this build is Wayland-only (Omarchy / Hyprland)"
                    .into(),
            ));
        }
        let wayshot = match WayshotConnection::new() {
            Ok(c) => {
                tracing::info!("wlr-screencopy (libwayshot) ready");
                Some(c)
            }
            Err(e) => {
                tracing::warn!("libwayshot unavailable ({e}); will use grim if present");
                None
            }
        };
        let this = Self {
            wayshot: Mutex::new(wayshot),
            cached_screens: Mutex::new(Vec::new()),
        };
        match this.screens() {
            Ok(s) => *this.lock_screens() = s,
            Err(e) => tracing::warn!("initial screen enumeration failed: {e}; will retry on use"),
        }
        Ok(this)
    }

    fn lock_wayshot(&self) -> std::sync::MutexGuard<'_, Option<WayshotConnection>> {
        self.wayshot.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_screens(&self) -> std::sync::MutexGuard<'_, Vec<Screen>> {
        self.cached_screens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn screens(&self) -> Result<Vec<Screen>> {
        // hyprctl tracks mode/fullscreen output changes; a long-lived
        // libwayshot connection's output list can go stale or hang.
        match hypr_monitors() {
            Ok(s) => Ok(s),
            Err(hypr_err) => {
                if let Some(ws) = self.lock_wayshot().as_ref() {
                    let outs = ws.get_all_outputs();
                    if !outs.is_empty() {
                        return Ok(outs.iter().map(screen_from_output).collect());
                    }
                }
                Err(hypr_err)
            }
        }
    }

    fn cursor_position(&self) -> Result<(i32, i32)> {
        hypr_cursor().or(Ok((0, 0)))
    }

    pub fn cursor_screen(&self) -> Result<Screen> {
        let screens = {
            let fresh = self.screens()?;
            *self.lock_screens() = fresh.clone();
            fresh
        };
        if screens.len() == 1 {
            return Ok(screens.into_iter().next().unwrap());
        }
        if let Ok(s) = hypr_focused_monitor(&screens) {
            return Ok(s);
        }
        let (x, y) = self.cursor_position()?;
        if let Some(s) = screens
            .iter()
            .find(|s| x >= s.x && y >= s.y && x < s.x + s.width as i32 && y < s.y + s.height as i32)
        {
            return Ok(s.clone());
        }
        screens
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("no monitors found".into()))
    }

    pub fn capture_all(&self) -> Result<RgbaImage> {
        self.capture_all_with_cursor(false)
    }

    pub fn capture_screen_with_cursor(
        &self,
        screen: &Screen,
        include_cursor: bool,
    ) -> Result<RgbaImage> {
        if let Some(img) = self.wayshot_one(screen, include_cursor) {
            return Ok(img);
        }
        grim_output(&screen.name, include_cursor)
    }

    fn capture_all_with_cursor(&self, include_cursor: bool) -> Result<RgbaImage> {
        if let Some(img) = self.wayshot_all(include_cursor) {
            return Ok(img);
        }
        grim_all(include_cursor)
    }

    fn reconnect_wayshot(&self) {
        let conn = match WayshotConnection::new() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("libwayshot reconnect failed: {e}");
                None
            }
        };
        *self.lock_wayshot() = conn;
    }

    /// Take the live connection out so a hung screenshot cannot hold the mutex.
    fn take_wayshot(&self) -> Option<WayshotConnection> {
        self.lock_wayshot().take()
    }

    fn restore_wayshot(&self, ws: WayshotConnection) {
        *self.lock_wayshot() = Some(ws);
    }

    fn wayshot_one(&self, screen: &Screen, include_cursor: bool) -> Option<RgbaImage> {
        let ws = self.take_wayshot()?;
        let name = screen.name.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        if let Err(e) = std::thread::Builder::new()
            .name("wayshot-one".into())
            .spawn(move || {
                let hit = ws
                    .get_all_outputs()
                    .iter()
                    .find(|o| o.name == name)
                    .cloned();
                let result = match hit {
                    Some(out) => ws
                        .screenshot_single_output(&out, include_cursor)
                        .map(|img| img.to_rgba8()),
                    None => Err(libwayshot::Error::NoOutputs),
                };
                let _ = tx.send((result, ws));
            })
        {
            tracing::warn!("libwayshot thread: {e}");
            self.reconnect_wayshot();
            return None;
        }
        match rx.recv_timeout(WAYSHOT_TIMEOUT) {
            Ok((Ok(img), ws)) => {
                self.restore_wayshot(ws);
                Some(img)
            }
            Ok((Err(e), _ws)) => {
                tracing::warn!("libwayshot: {e}; falling back to grim");
                self.reconnect_wayshot();
                None
            }
            Err(_) => {
                tracing::warn!("libwayshot timed out; falling back to grim");
                self.reconnect_wayshot();
                None
            }
        }
    }

    fn wayshot_all(&self, include_cursor: bool) -> Option<RgbaImage> {
        let ws = self.take_wayshot()?;
        let (tx, rx) = mpsc::sync_channel(1);
        if let Err(e) = std::thread::Builder::new()
            .name("wayshot-all".into())
            .spawn(move || {
                let result = ws.screenshot_all(include_cursor).map(|img| img.to_rgba8());
                let _ = tx.send((result, ws));
            })
        {
            tracing::warn!("libwayshot thread: {e}");
            self.reconnect_wayshot();
            return None;
        }
        match rx.recv_timeout(WAYSHOT_TIMEOUT) {
            Ok((Ok(img), ws)) => {
                self.restore_wayshot(ws);
                Some(img)
            }
            Ok((Err(e), _ws)) => {
                tracing::warn!("libwayshot screenshot_all: {e}; falling back to grim");
                self.reconnect_wayshot();
                None
            }
            Err(_) => {
                tracing::warn!("libwayshot screenshot_all timed out; falling back to grim");
                self.reconnect_wayshot();
                None
            }
        }
    }
}

fn screen_from_output(o: &libwayshot::output::OutputInfo) -> Screen {
    let pos = o.logical_position();
    let size = o.physical_size();
    Screen {
        x: pos.x as i32,
        y: pos.y as i32,
        width: size.width,
        height: size.height,
        name: o.name.clone(),
    }
}

fn grim_output(name: &str, cursor: bool) -> Result<RgbaImage> {
    if name.is_empty() {
        return grim_all(cursor);
    }
    grim_ppm(Some(name), cursor)
}

fn grim_all(cursor: bool) -> Result<RgbaImage> {
    grim_ppm(None, cursor)
}

fn grim_ppm(output: Option<&str>, cursor: bool) -> Result<RgbaImage> {
    let mut cmd = Command::new("grim");
    cmd.args(["-t", "ppm"]);
    if let Some(name) = output {
        cmd.args(["-o", name]);
    }
    if cursor {
        cmd.arg("-c");
    }
    cmd.arg("-");
    run_grim(cmd)
}

fn run_grim(mut cmd: Command) -> Result<RgbaImage> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("grim: {e}")))?;
    let pid = child.id();
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("grim-wait".into())
        .spawn(move || {
            let _ = tx.send(child.wait_with_output());
        })
        .map_err(|e| Error::Other(format!("grim wait thread: {e}")))?;
    let out = match rx.recv_timeout(GRIM_TIMEOUT) {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(Error::Other(format!("grim: {e}"))),
        Err(_) => {
            let _ = Command::new("kill").arg(pid.to_string()).status();
            return Err(Error::Other("grim timed out".into()));
        }
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(Error::Other(format!("grim failed: {err}")));
    }
    let reader = ImageReader::new(Cursor::new(out.stdout)).with_guessed_format()?;
    Ok(reader.decode()?.to_rgba8())
}

fn hypr_monitors() -> Result<Vec<Screen>> {
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .map_err(|e| Error::Other(format!("hyprctl monitors: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other("hyprctl monitors failed".into()));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Other(format!("hyprctl monitors json: {e}")))?;
    let arr = v
        .as_array()
        .ok_or_else(|| Error::Other("hyprctl monitors: expected array".into()))?;
    let mut screens = Vec::new();
    for m in arr {
        let name = m["name"].as_str().unwrap_or("").to_string();
        let x = m["x"].as_i64().unwrap_or(0) as i32;
        let y = m["y"].as_i64().unwrap_or(0) as i32;
        let width = m["width"].as_u64().unwrap_or(0) as u32;
        let height = m["height"].as_u64().unwrap_or(0) as u32;
        if width == 0 || height == 0 {
            continue;
        }
        screens.push(Screen {
            x,
            y,
            width,
            height,
            name,
        });
    }
    if screens.is_empty() {
        return Err(Error::Other("hyprctl returned no monitors".into()));
    }
    Ok(screens)
}

fn hypr_focused_monitor(screens: &[Screen]) -> Result<Screen> {
    // Prefer a cached match: `focused` via hyprctl is one extra process.
    // Only used if layout is ambiguous.
    let out = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .map_err(|e| Error::Other(format!("hyprctl monitors: {e}")))?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::Other(format!("hyprctl monitors json: {e}")))?;
    if let Some(arr) = v.as_array() {
        for m in arr {
            if m["focused"].as_bool() == Some(true) {
                let name = m["name"].as_str().unwrap_or("");
                if let Some(s) = screens.iter().find(|s| s.name == name) {
                    return Ok(s.clone());
                }
            }
        }
    }
    Err(Error::Other("no focused monitor".into()))
}

fn hypr_cursor() -> Result<(i32, i32)> {
    let out = Command::new("hyprctl")
        .arg("cursorpos")
        .output()
        .map_err(|e| Error::Other(format!("hyprctl cursorpos: {e}")))?;
    let s = String::from_utf8_lossy(&out.stdout);
    let mut parts = s.split(',');
    let x = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .ok_or_else(|| Error::Other(format!("bad cursorpos: {s}")))?;
    let y = parts
        .next()
        .and_then(|p| p.trim().parse().ok())
        .ok_or_else(|| Error::Other(format!("bad cursorpos: {s}")))?;
    Ok((x, y))
}
