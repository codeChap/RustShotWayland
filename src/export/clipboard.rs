use crate::error::{Error, Result};
use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder, RgbaImage};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Put `img` on the Wayland clipboard as `image/png` via `wl-copy`.
pub fn copy(img: &RgbaImage, also_write_latest: Option<&Path>) -> Result<()> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| Error::Other(format!("png encode: {e}")))?;

    spawn_wl_copy(&png)?;

    if let Some(latest) = also_write_latest {
        match write_latest(latest, &png) {
            Ok(()) => tracing::info!(path = %latest.display(), "wrote latest screenshot copy"),
            Err(e) => tracing::warn!(
                path = %latest.display(),
                "failed to write latest screenshot copy: {e}"
            ),
        }
    }

    Ok(())
}

fn write_latest(path: &Path, png: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, png)
}

fn spawn_wl_copy(png: &[u8]) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            Error::Other(format!(
                "spawn wl-copy: {e} — install wl-clipboard (Omarchy ships it)"
            ))
        })?;

    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("wl-copy stdin unavailable".into()))?;
        stdin
            .write_all(png)
            .map_err(|e| Error::Other(format!("write to wl-copy: {e}")))?;
    }

    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}
