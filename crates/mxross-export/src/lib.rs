// crates/mxross-export/src/lib.rs
//! Encodes raw pixel buffers into output file formats. Currently just
//! PNG — MidManStudio's own MPX raster format is a natural future
//! addition here, alongside PNG rather than instead of it.
//!
//! Deliberately knows nothing about wgpu, Android, or any other
//! platform/rendering concern — it only ever sees plain bytes plus
//! width/height/color.

/// Encodes `rgba` (tightly packed, 4 bytes per pixel, row-major,
/// `width * height * 4` bytes total) as a PNG with a full alpha channel.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let expected_len = (width as usize) * (height as usize) * 4;
    if rgba.len() != expected_len {
        return Err(format!(
            "encode_png: expected {expected_len} bytes for a {width}x{height} RGBA8 image, got {}",
            rgba.len()
        ));
    }

    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("PNG header write failed: {e}"))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("PNG data write failed: {e}"))?;
    }
    Ok(bytes)
}

/// Composites `rgba` onto a solid `background` color, producing a fully
/// opaque buffer of the same dimensions (alpha 255 everywhere). Standard
/// non-premultiplied "alpha over" math, matching how the canvas itself
/// stores color (straight alpha, not premultiplied).
///
/// Used when the canvas's background mode is "solid" rather than
/// "transparent" — the export should match whatever's currently showing
/// in the live preview, not silently always produce transparency
/// regardless of that setting.
pub fn flatten_onto(rgba: &[u8], background: [u8; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for px in rgba.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        let blend = |fg: u8, bg: u8| -> u8 {
            (fg as f32 * a + bg as f32 * (1.0 - a)).round() as u8
        };
        out.push(blend(px[0], background[0]));
        out.push(blend(px[1], background[1]));
        out.push(blend(px[2], background[2]));
        out.push(255);
    }
    out
            }
