pub mod grim;

pub use self::grim::WaylandCapture;

/// Uninitialized pixel buffer. Caller must write every byte before reading.
pub(crate) fn alloc_pixels(n: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(n);
    // SAFETY: length is set to `n` of uninit memory; every caller fills the
    // whole buffer (swizzle or dim) before the Vec is read or wrapped in RgbaImage.
    unsafe {
        v.set_len(n);
    }
    v
}

/// Swap R↔B and force alpha 0xFF. Turns RGBA into Wayland Argb8888 (BGRA little-endian).
#[inline]
fn swizzle_u32(px: u32) -> u32 {
    (px & 0xFF00_FF00)
        | ((px & 0x00FF_0000) >> 16)
        | ((px & 0x0000_00FF) << 16)
        | 0xFF00_0000
}

#[inline]
pub(crate) fn swizzle_rb_opaque(src: &[u8], dst: &mut [u8]) {
    debug_assert_eq!(src.len(), dst.len());
    debug_assert_eq!(src.len() % 4, 0);
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
        let px = u32::from_ne_bytes([s[0], s[1], s[2], s[3]]);
        d.copy_from_slice(&swizzle_u32(px).to_ne_bytes());
    }
}

/// Swizzle a dirty rectangle (RGBA src → BGRA dst), both stride = `width` pixels.
#[inline]
pub(crate) fn swizzle_rb_opaque_rect(
    src: &[u8],
    dst: &mut [u8],
    width: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) {
    let stride = (width as usize) * 4;
    let x0 = x as usize;
    let wpx = w as usize;
    for row in 0..(h as usize) {
        let off = (y as usize + row) * stride + x0 * 4;
        let end = off + wpx * 4;
        if end > src.len() || end > dst.len() {
            break;
        }
        swizzle_rb_opaque(&src[off..end], &mut dst[off..end]);
    }
}

#[derive(Debug, Clone)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Hyprland / wlr output name (`eDP-1`, `HDMI-A-1`). Empty if unknown.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_rb_opaque_swaps_red_blue_and_forces_alpha() {
        let src = [0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0x00];
        let mut dst = [0u8; 8];
        swizzle_rb_opaque(&src, &mut dst);
        assert_eq!(dst, [0x33, 0x22, 0x11, 0xff, 0xcc, 0xbb, 0xaa, 0xff]);
    }
}
