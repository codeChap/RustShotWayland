//! In-process wlr-screencopy via libwayshot, with `grim` as fallback.

use super::Screen;
use crate::error::{Error, Result};
use image::{ImageReader, RgbaImage};
use libwayshot::WayshotConnection;
use std::io::Cursor;
use std::process::Command;
use std::sync::Mutex;

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
            Ok(s) => *this.cached_screens.lock().unwrap() = s,
            Err(e) => tracing::warn!("initial screen enumeration failed: {e}; will retry on use"),
        }
        Ok(this)
    }

    pub fn screens(&self) -> Result<Vec<Screen>> {
        if let Some(ws) = self.wayshot.lock().unwrap().as_ref() {
            let outs = ws.get_all_outputs();
            if !outs.is_empty() {
                return Ok(outs.iter().map(screen_from_output).collect());
            }
        }
        hypr_monitors()
    }

    fn cursor_position(&self) -> Result<(i32, i32)> {
        hypr_cursor().or(Ok((0, 0)))
    }

    pub fn cursor_screen(&self) -> Result<Screen> {
        let screens = {
            let cached = self.cached_screens.lock().unwrap();
            if cached.is_empty() {
                drop(cached);
                let fresh = self.screens()?;
                *self.cached_screens.lock().unwrap() = fresh.clone();
                fresh
            } else {
                cached.clone()
            }
        };
        if screens.len() == 1 {
            return Ok(screens.into_iter().next().unwrap());
        }
        if let Ok(s) = hypr_focused_monitor(&screens) {
            return Ok(s);
        }
        let (x, y) = self.cursor_position()?;
        if let Some(s) = screens.iter().find(|s| {
            x >= s.x && y >= s.y && x < s.x + s.width as i32 && y < s.y + s.height as i32
        }) {
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
            return img;
        }
        grim_output(&screen.name, include_cursor)
    }

    fn capture_all_with_cursor(&self, include_cursor: bool) -> Result<RgbaImage> {
        {
            let guard = self.wayshot.lock().unwrap();
            if let Some(ws) = guard.as_ref() {
                match ws.screenshot_all(include_cursor) {
                    Ok(img) => return Ok(img.to_rgba8()),
                    Err(e) => tracing::warn!("libwayshot screenshot_all: {e}"),
                }
            }
        }
        grim_all(include_cursor)
    }

    fn wayshot_one(&self, screen: &Screen, include_cursor: bool) -> Option<Result<RgbaImage>> {
        let guard = self.wayshot.lock().unwrap();
        let ws = guard.as_ref()?;
        let hit = ws
            .get_all_outputs()
            .iter()
            .find(|o| o.name == screen.name)
            .cloned();
        let Some(out) = hit else {
            return None;
        };
        Some(
            ws.screenshot_single_output(&out, include_cursor)
                .map(|img| img.to_rgba8())
                .map_err(|e| Error::Other(format!("libwayshot: {e}"))),
        )
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
    let mut cmd = Command::new("grim");
    cmd.args(["-t", "ppm", "-o", name]);
    if cursor {
        cmd.arg("-c");
    }
    cmd.arg("-");
    run_grim(cmd)
}

fn grim_all(cursor: bool) -> Result<RgbaImage> {
    let mut cmd = Command::new("grim");
    cmd.args(["-t", "ppm"]);
    if cursor {
        cmd.arg("-c");
    }
    cmd.arg("-");
    run_grim(cmd)
}

fn run_grim(mut cmd: Command) -> Result<RgbaImage> {
    let out = cmd.output().map_err(|e| Error::Other(format!("grim: {e}")))?;
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
