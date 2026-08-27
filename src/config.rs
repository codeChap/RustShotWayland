use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub defaults: Defaults,
    pub capture: CaptureCfg,
    pub clipboard: Clipboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub counter_radius: f32,
    pub pixelate_block: u32,
    pub save_dir: String,
    pub filename_pattern: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            counter_radius: 16.0,
            pixelate_block: 10,
            save_dir: "~/Pictures/screenshots".into(),
            filename_pattern: "%Y%m%d-%H%M%S.png".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CaptureCfg {
    pub include_cursor: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Clipboard {
    /// When copying to the clipboard, also write a copy of the final image to
    /// this path on disk. This is the reliable way to paste screenshots into AI
    /// tools that don't read `image/png` from the clipboard well — reference the
    /// file with `@~/Pictures/screenshots/rustshot-latest.png`. Empty disables it.
    ///
    /// The path is intentionally not also placed on the clipboard — `wl-copy`
    /// is invoked as `image/png` only so pasting still yields the image.
    pub latest_path: String,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            latest_path: "~/Pictures/screenshots/rustshot-latest.png".into(),
        }
    }
}

impl Config {
    pub fn load_or_default() -> Self {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<Self>(&s) {
                Ok(c) => {
                    tracing::info!(path = %path.display(), "loaded config");
                    c
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "invalid config; using defaults");
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file; using defaults");
                Self::default()
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "config read error; using defaults");
                Self::default()
            }
        }
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustshot-wayland")
        .join("config.toml")
}

/// Build an auto-save path from `save_dir` (with `~` expanded) and a strftime-style filename pattern.
pub fn auto_save_path(save_dir: &str, pattern: &str) -> PathBuf {
    let dir = expand_tilde(save_dir);
    let now = chrono::Local::now();
    let fname = now.format(pattern).to_string();
    dir.join(fname)
}

pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if p == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(p)
}
