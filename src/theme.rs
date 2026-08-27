//! Overlay chrome palette.
//!
//! On Omarchy, this is the live theme at
//! `~/.local/state/omarchy/current/theme/colors.toml` (re-read each overlay so
//! a theme switch applies to the next capture without restarting the daemon).
//! Annotation strokes stay `canvas::Style::default` (red) — this file is UI
//! chrome only. Off Omarchy, [`Theme::fallback`] matches the original yellow
//! frame / dark strip.

use image::Rgba;
use std::collections::HashMap;
use std::path::PathBuf;
use tiny_skia::Color;

/// Semantic chrome colors. All channels are RGBA, alpha usually 255.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: [u8; 4],
    pub background: [u8; 4],
    pub lighter_background: [u8; 4],
    pub selection: [u8; 4],
    pub muted: [u8; 4],
    pub foreground: [u8; 4],
    pub bright_foreground: [u8; 4],
    pub dark: bool,
}

impl Theme {
    /// Pre-Omarchy chrome: yellow selection, dark strip.
    pub fn fallback() -> Self {
        Self {
            accent: [255, 200, 0, 255],
            background: [28, 28, 32, 255],
            lighter_background: [48, 48, 56, 255],
            selection: [64, 64, 72, 255],
            muted: [90, 90, 108, 255],
            foreground: [210, 210, 210, 255],
            bright_foreground: [255, 255, 255, 255],
            dark: true,
        }
    }

    pub fn load() -> Self {
        let path = colors_path();
        match load_from_path(&path) {
            Some(t) => {
                tracing::info!(path = %path.display(), "loaded Omarchy theme chrome");
                t
            }
            None => {
                tracing::debug!(
                    path = %path.display(),
                    "no Omarchy colors.toml; using fallback chrome"
                );
                Self::fallback()
            }
        }
    }

    pub fn strip_bg(self) -> [u8; 4] {
        let mut c = self.background;
        c[3] = 230;
        c
    }

    pub fn handle_fill(self) -> [u8; 4] {
        if self.dark {
            [255, 255, 255, 255]
        } else {
            self.background
        }
    }

    pub fn on_accent(self) -> [u8; 4] {
        contrast_ink(self.accent)
    }
}

pub fn skia(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

pub fn rgba(c: [u8; 4]) -> Rgba<u8> {
    Rgba(c)
}

fn colors_path() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("omarchy/current/theme/colors.toml")
}

fn load_from_path(path: &std::path::Path) -> Option<Theme> {
    let s = std::fs::read_to_string(path).ok()?;
    parse_colors_toml(&s)
}

fn parse_colors_toml(s: &str) -> Option<Theme> {
    let table: HashMap<String, toml::Value> = toml::from_str(s).ok()?;
    let get = |keys: &[&str]| -> Option<[u8; 4]> {
        for k in keys {
            if let Some(v) = table.get(*k) {
                if let Some(hex) = v.as_str() {
                    if let Some(c) = parse_hex(hex) {
                        return Some(c);
                    }
                }
            }
        }
        None
    };

    let accent = get(&["accent"])?;
    let background = get(&["background", "bg"]).unwrap_or([28, 28, 32, 255]);
    let lighter_background =
        get(&["lighter_background", "lighter_bg"]).unwrap_or(background);
    let selection = get(&["selection"]).unwrap_or(lighter_background);
    let muted = get(&["muted"]).unwrap_or([90, 90, 108, 255]);
    let foreground = get(&["foreground", "fg"]).unwrap_or([210, 210, 210, 255]);
    let bright_foreground =
        get(&["bright_foreground", "bright_fg"]).unwrap_or(foreground);
    let mode = table
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("dark");
    let dark = !mode.eq_ignore_ascii_case("light");

    Some(Theme {
        accent,
        background,
        lighter_background,
        selection,
        muted,
        foreground,
        bright_foreground,
        dark,
    })
}

fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let t = s.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    match t.len() {
        3 => {
            let r = u8::from_str_radix(&t[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&t[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&t[2..3].repeat(2), 16).ok()?;
            Some([r, g, b, 255])
        }
        6 => {
            let r = u8::from_str_radix(&t[0..2], 16).ok()?;
            let g = u8::from_str_radix(&t[2..4], 16).ok()?;
            let b = u8::from_str_radix(&t[4..6], 16).ok()?;
            Some([r, g, b, 255])
        }
        8 => {
            let r = u8::from_str_radix(&t[0..2], 16).ok()?;
            let g = u8::from_str_radix(&t[2..4], 16).ok()?;
            let b = u8::from_str_radix(&t[4..6], 16).ok()?;
            let a = u8::from_str_radix(&t[6..8], 16).ok()?;
            Some([r, g, b, a])
        }
        _ => None,
    }
}

fn contrast_ink(bg: [u8; 4]) -> [u8; 4] {
    if relative_luma(bg) > 0.45 {
        [20, 20, 24, 255]
    } else {
        [255, 255, 255, 255]
    }
}

fn relative_luma(c: [u8; 4]) -> f32 {
    fn lin(v: u8) -> f32 {
        let s = v as f32 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_rrggbb() {
        assert_eq!(parse_hex("#e68e0d"), Some([0xe6, 0x8e, 0x0d, 255]));
        assert_eq!(parse_hex("89b4fa"), Some([0x89, 0xb4, 0xfa, 255]));
    }

    #[test]
    fn parse_hex_short_and_alpha() {
        assert_eq!(parse_hex("#abc"), Some([0xaa, 0xbb, 0xcc, 255]));
        assert_eq!(parse_hex("#11223344"), Some([0x11, 0x22, 0x33, 0x44]));
    }

    #[test]
    fn omarchy_colors_toml_uses_accent_for_chrome() {
        let toml = r##"
mode = "dark"
accent = "#e68e0d"
selection = "#2a2a2a"
muted = "#333333"
background = "#121212"
lighter_background = "#1e1e1e"
foreground = "#bebebe"
bright_foreground = "#eaeaea"
"##;
        let t = parse_colors_toml(toml).expect("parse");
        assert_eq!(t.accent, [0xe6, 0x8e, 0x0d, 255]);
        assert_eq!(t.background, [0x12, 0x12, 0x12, 255]);
        assert_eq!(t.muted, [0x33, 0x33, 0x33, 255]);
        assert!(t.dark);
        assert_eq!(t.strip_bg()[3], 230);
    }

    #[test]
    fn light_mode_and_legacy_bg_fg_keys() {
        let toml = r##"
mode = "light"
accent = "#89b4fa"
bg = "#eff1f5"
fg = "#4c4f69"
"##;
        let t = parse_colors_toml(toml).expect("parse");
        assert!(!t.dark);
        assert_eq!(t.background, [0xef, 0xf1, 0xf5, 255]);
        assert_eq!(t.foreground, [0x4c, 0x4f, 0x69, 255]);
        assert_eq!(t.handle_fill(), t.background);
    }

    #[test]
    fn missing_accent_is_not_a_theme() {
        let toml = "mode = \"dark\"\nbackground = \"#000000\"\n";
        assert!(parse_colors_toml(toml).is_none());
    }

    #[test]
    fn fallback_keeps_original_yellow_frame() {
        let t = Theme::fallback();
        assert_eq!(t.accent, [255, 200, 0, 255]);
        assert_eq!(t.background, [28, 28, 32, 255]);
    }

    #[test]
    fn contrast_ink_picks_dark_on_yellow() {
        assert_eq!(contrast_ink([255, 200, 0, 255]), [20, 20, 24, 255]);
        assert_eq!(contrast_ink([18, 18, 18, 255]), [255, 255, 255, 255]);
    }
}
